//! Drawing Flow, and the bar at the top that names both views. The bar lies outside the element
//! whose listeners pause follow, and its controls call `steered`, never `acted`.

use std::ops::Range;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yaar_link::{FlowForm, Step, TurnEnd};

use crate::draw::{InView, ROW_HEIGHT};
use crate::{Following, ReviewPane, TimelineEntry, ToggleFollow, View};

impl ReviewPane {
    pub(crate) fn render_bar(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let theme = cx.theme();
        // Runs `then` on mouse down and never calls `acted`: the bar chooses what Flow shows, it
        // does not read it.
        let control =
            |label: &str,
             selector: &'static str,
             marked: bool,
             then: fn(&mut ReviewPane, &mut Window, &mut Context<ReviewPane>)| {
                div()
                    .debug_selector(|| selector.to_owned())
                    .flex_none()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(marked, |name| {
                        name.bg(theme.secondary)
                            .text_color(theme.secondary_foreground)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, _: &MouseDownEvent, window, cx| {
                            pane.steered(cx);
                            then(pane, window, cx);
                        }),
                    )
                    .child(label.to_owned())
            };
        let in_flow = self.view == View::Flow;
        let follow = match self.following {
            Following::On => "Follow: on",
            Following::Paused { .. } => "Follow: paused",
            Following::Off => "Follow: off",
        };
        let forms = in_flow.then(|| {
            div()
                .flex_none()
                .flex()
                .gap_1()
                .child(control(
                    "Timeline",
                    "flow-form-timeline",
                    self.form == FlowForm::Timeline,
                    |pane, _, cx| pane.choose_form(FlowForm::Timeline, cx),
                ))
                .child(control(
                    "Live diff",
                    "flow-form-live",
                    self.form == FlowForm::LiveDiff,
                    |pane, _, cx| pane.choose_form(FlowForm::LiveDiff, cx),
                ))
        });
        Some(
            div()
                .debug_selector(|| "review-bar".to_owned())
                .flex_none()
                .h(ROW_HEIGHT)
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .overflow_hidden()
                .whitespace_nowrap()
                .border_b_1()
                .border_color(theme.border)
                // The focus goes to the rows, where the keys of the pane work.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                        pane.focus.focus(window, cx);
                    }),
                )
                .child(control(
                    "Changes",
                    "review-view-changes",
                    !in_flow,
                    |pane, _, cx| pane.show_view(View::Changes, cx),
                ))
                .child(control(
                    "Flow",
                    "review-view-flow",
                    in_flow,
                    |pane, _, cx| pane.show_view(View::Flow, cx),
                ))
                // Keeps the middle of the bar free of controls: what is right of it is Flow's own.
                .child(div().flex_1())
                .children(forms)
                .child(
                    div()
                        .debug_selector(|| "flow-follow".to_owned())
                        .flex_none()
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(theme.muted_foreground)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                                pane.toggle_follow(&ToggleFollow, window, cx);
                            }),
                        )
                        .child(follow),
                ),
        )
    }

    pub(crate) fn render_flow(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let lines = self.follow_lines(cx);
        let list = match self.form {
            FlowForm::Timeline => self.render_timeline(cx),
            FlowForm::LiveDiff => div()
                .debug_selector(|| "flow-live".to_owned())
                .size_full()
                .child(self.render_live(cx))
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(lines)
            .child(div().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    fn render_timeline(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.flow.entries().is_empty() {
            let theme = cx.theme();
            return div()
                .debug_selector(|| "flow-empty".to_owned())
                .h(ROW_HEIGHT)
                .px_2()
                .flex()
                .items_center()
                .text_color(theme.muted_foreground)
                .child("No turn of the agent to show yet")
                .into_any_element();
        }
        uniform_list(
            "flow-timeline",
            self.flow.entries().len(),
            cx.processor(|pane, rows: Range<usize>, _, cx| {
                let cursor = pane.flow.cursor();
                let at = pane.at.as_ref().and_then(|at| pane.flow.row_of(at));
                rows.map(|index| {
                    pane.draw_timeline_entry(index, cursor == Some(index), at == Some(index), cx)
                })
                .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.flow.scroll)
        .with_decoration(InView(self.flow.in_view.clone()))
        .size_full()
        .into_any_element()
    }

    fn draw_timeline_entry(
        &self,
        index: usize,
        reviewer: bool,
        at: bool,
        cx: &Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let words = entry_words(&self.flow.entries()[index]);
        let heading = matches!(self.flow.entries()[index], TimelineEntry::Turn { .. });
        div()
            .debug_selector(|| format!("flow-entry-{index}"))
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_l_2()
            .border_color(if at {
                theme.primary
            } else {
                transparent_black()
            })
            .when(heading, |row| {
                row.bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
            })
            .when(reviewer && !heading, |row| row.bg(theme.muted))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                    pane.click_timeline_entry(index, cx)
                }),
            )
            .child(
                // The words are in the selector: a test reads what was handed to the text system.
                div()
                    .debug_selector({
                        let words = words.clone();
                        move || format!("flow-entry-text-{index}:{words}")
                    })
                    .truncate()
                    .child(words),
            )
    }
}

fn entry_words(entry: &TimelineEntry) -> String {
    match entry {
        TimelineEntry::Turn { title, .. } => title.clone(),
        TimelineEntry::Step { step, .. } => match step {
            Step::Read { path } => format!("Read {}", path.display()),
            Step::Edit { path, lines } => {
                let lines =
                    lines
                        .iter()
                        .flatten()
                        .map(|range| match range.start() == range.end() {
                            true => range.start().to_string(),
                            false => format!("{}-{}", range.start(), range.end()),
                        });
                let lines = lines.collect::<Vec<_>>().join(", ");
                match lines.is_empty() {
                    true => format!("Edit {}", path.display()),
                    false => format!("Edit {} lines {lines}", path.display()),
                }
            }
            Step::Write { path } => format!("Write {}", path.display()),
            Step::Command { command } => {
                format!("Run {}", command.lines().next().unwrap_or_default())
            }
        },
        TimelineEntry::End { how, .. } => match how {
            Some(TurnEnd::Finished) => "Turn finished".to_owned(),
            Some(TurnEnd::Interrupted) => "Turn interrupted".to_owned(),
            None => "Turn ended".to_owned(),
        },
    }
}
