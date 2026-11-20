//! The question an Adapter asks before it writes into the user's own settings of its CLI,
//! and the user's answer.
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

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{Consent, ConsentAnswer};

use crate::{InstallAgentExtensions, KeepAgentExtensionsOut, TerminalPane};

/// The notice when the user answers and no Adapter needs leave.
pub const NOTHING_TO_INSTALL: &str = "No agent CLI asks to install anything.";

impl TerminalPane {
    /// The questions the pane shows, as the Keeper told them: the Adapter's own words with
    /// what went wrong, in the order of the Adapters. Empty while no line is drawn.
    pub fn questions(&self, cx: &App) -> Vec<Consent> {
        self.models.consents.read(cx).asked()
    }

    /// The line of the module's doc, while a question is shown.
    pub(crate) fn render_consent(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let asked = self.questions(cx);
        if asked.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let questions = asked.into_iter().enumerate().map(|(n, consent)| {
            // Each text takes the width of its words, not the line's, so that its bounds
            // show what is drawn in it.
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_0p5()
                .child(
                    div()
                        .debug_selector(move || format!("terminal-consent-question-{n}"))
                        .text_sm()
                        .child(consent.question),
                )
                .children(consent.problem.map(|problem| {
                    div()
                        .debug_selector(move || format!("terminal-consent-problem-{n}"))
                        .text_sm()
                        .text_color(theme.danger)
                        .child(problem)
                }))
        });
        let control = |name: &'static str, label: &'static str, primary: bool| {
            let button = div()
                .id(SharedString::from(name))
                .debug_selector(move || name.to_owned())
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
        Some(
            div()
                .debug_selector(|| "terminal-consent".to_owned())
                .flex_none()
                .px_3()
                .py_1()
                .gap_1()
                .flex()
                .flex_col()
                .bg(theme.muted)
                .text_color(theme.foreground)
                .border_b_1()
                .border_color(theme.border)
                .children(questions)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(control("terminal-consent-yes", "Install", true).on_click(
                            cx.listener(|pane, _: &ClickEvent, _, cx| {
                                pane.answer(ConsentAnswer::Yes, cx)
                            }),
                        ))
                        .child(
                            control("terminal-consent-no", "No", false).on_click(cx.listener(
                                |pane, _: &ClickEvent, _, cx| pane.answer(ConsentAnswer::No, cx),
                            )),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Sends the user's `answer` to the questions for leave, through the shared model, and
    /// says in the notice when there is none to answer or the Keeper refused. The notice is
    /// the pane's one, so the newest answer's outcome replaces whatever it said before, a
    /// terminal's failure included.
    fn answer(&mut self, answer: ConsentAnswer, cx: &mut Context<Self>) {
        self.notice = None;
        let task = self
            .models
            .consents
            .update(cx, |consents, cx| consents.answer_all(answer, cx));
        // Detached, not kept: a task that is dropped is cancelled, and its answers lost.
        cx.spawn(async move |pane, cx| {
            let outcome = task.await;
            pane.update(cx, |pane, cx| {
                match outcome {
                    Ok(true) => {}
                    Ok(false) => pane.notice = Some(NOTHING_TO_INSTALL.to_owned()),
                    Err(error) => pane.notice = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// What [`crate::InstallAgentExtensions`] does.
    pub(crate) fn install_agent_extensions(
        &mut self,
        _: &InstallAgentExtensions,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer(ConsentAnswer::Yes, cx);
    }

    /// What [`crate::KeepAgentExtensionsOut`] does.
    pub(crate) fn keep_agent_extensions_out(
        &mut self,
        _: &KeepAgentExtensionsOut,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer(ConsentAnswer::No, cx);
    }
}
