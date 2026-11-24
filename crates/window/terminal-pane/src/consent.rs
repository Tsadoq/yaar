//! The question an Adapter asks before it writes into the user's settings of its CLI, with
//! the yes and no controls. Not modal: terminals and launchers work while it is up.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{Consent, ConsentAnswer};

use crate::{InstallAgentExtensions, KeepAgentExtensionsOut, TerminalPane};

pub const YES_LABEL: &str = "Install";

pub const NO_LABEL: &str = "No";

pub const NOTHING_TO_INSTALL: &str = "No agent CLI asks to install anything.";

impl TerminalPane {
    /// The questions shown, as the Keeper told them, in the order of the Adapters.
    pub fn questions(&self, cx: &App) -> Vec<Consent> {
        self.models.consents.read(cx).asked()
    }

    pub(crate) fn render_consent(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let asked = self.questions(cx);
        if asked.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let questions = asked.into_iter().enumerate().map(|(n, consent)| {
            // Each text takes the width of its words, so that its bounds show what is drawn in it.
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
            // The selector carries the text drawn, so that a test reads which words a control
            // shows.
            let words = div()
                .debug_selector(move || format!("{name}-label:{label}"))
                .child(label);
            if primary {
                button
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .child(words)
            } else {
                button
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .child(words)
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
                        .child(control("terminal-consent-yes", YES_LABEL, true).on_click(
                            cx.listener(|pane, _: &ClickEvent, _, cx| {
                                pane.answer(ConsentAnswer::Yes, cx)
                            }),
                        ))
                        .child(control("terminal-consent-no", NO_LABEL, false).on_click(
                            cx.listener(|pane, _: &ClickEvent, _, cx| {
                                pane.answer(ConsentAnswer::No, cx)
                            }),
                        )),
                )
                .into_any_element(),
        )
    }

    /// Sends `answer` to every question; the notice says when there was none or the Keeper refused.
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

    pub(crate) fn install_agent_extensions(
        &mut self,
        _: &InstallAgentExtensions,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer(ConsentAnswer::Yes, cx);
    }

    pub(crate) fn keep_agent_extensions_out(
        &mut self,
        _: &KeepAgentExtensionsOut,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer(ConsentAnswer::No, cx);
    }
}
