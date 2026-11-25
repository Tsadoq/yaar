//! The question an Adapter asks before it writes into the user's settings of its CLI, with
//! the yes and no controls. Not modal: terminals and launchers work while it is up.

use gpui_kit::*;
use yaar_link::{Consent, ConsentAnswer};
use yaar_ui::metrics::{SPACE_1, SPACE_2, SPACE_3, SPACE_5};
use yaar_ui::pieces::{Button, ButtonKind};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

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
        let palette = palette(cx);
        let questions = asked.into_iter().enumerate().map(|(n, consent)| {
            // Each text takes the width of its words, so that its bounds show what is drawn in it.
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(SPACE_1)
                .child(
                    div()
                        .debug_selector(move || format!("terminal-consent-question-{n}"))
                        .typed(TypeRole::Body)
                        .text_color(palette.text)
                        .child(consent.question),
                )
                .children(consent.problem.map(|problem| {
                    div()
                        .debug_selector(move || format!("terminal-consent-problem-{n}"))
                        .typed(TypeRole::Body)
                        .text_color(palette.danger)
                        .child(problem)
                }))
        });
        let control = |name: &'static str, label: &'static str, primary: bool| {
            Button::new(name, label)
                .kind(if primary {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Secondary
                })
                .small()
                .selector(name)
                .label_selector(format!("{name}-label:{label}"))
        };
        Some(
            div()
                .debug_selector(|| "terminal-consent".to_owned())
                .flex_none()
                .px(SPACE_5)
                .py(SPACE_3)
                .gap(SPACE_2)
                .flex()
                .flex_col()
                .bg(palette.window)
                .text_color(palette.text)
                .border_b_1()
                .border_color(palette.separator)
                .children(questions)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(SPACE_3)
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
