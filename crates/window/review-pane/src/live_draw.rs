//! How the live diff of Flow is drawn: one list of rows of one height.

use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::{Row, Side};
use yaar_link::{Step, ThreadId};

use crate::draw::{
    DRAWN_BYTES, InView, NUMBER_WIDTH, ROW_HEIGHT, SIGN_WIDTH, side_name, status_words,
    unshown_sentence,
};
use crate::{Layout, PaneRow, ReviewPane, ShownDiff};

impl ReviewPane {
    pub(crate) fn render_live(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if self.live.change().is_none() {
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
        let activity = self.activity().map(|at| {
            let words = match self.flow.step(at) {
                Some(Step::Command { command }) => {
                    format!("Run {}", command.lines().next().unwrap_or_default())
                }
                Some(Step::Edit { path, .. }) => format!("Edit {}", path.display()),
                Some(Step::Write { path }) => format!("Write {}", path.display()),
                Some(Step::Read { path }) => format!("Read {}", path.display()),
                None => "The newest step".to_owned(),
            };
            div()
                .debug_selector(|| "flow-activity".to_owned())
                .flex_none()
                .h(ROW_HEIGHT)
                .w_full()
                .px_2()
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .border_b_1()
                .border_color(theme.border)
                .text_color(theme.muted_foreground)
                .child(words)
        });
        let list = if self.live.rows().is_empty() {
            div().into_any_element()
        } else {
            uniform_list(
                "flow-live-rows",
                self.live.rows().len(),
                cx.processor(|pane, rows: Range<usize>, _, cx| {
                    rows.map(|index| pane.draw_live_row(index, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.live.scroll)
            .with_decoration(InView(self.live.in_view.clone()))
            .size_full()
            .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(activity)
            .child(div().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    fn draw_live_row(&self, index: usize, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let row = div()
            .debug_selector(|| format!("flow-row-{index}"))
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap();
        match &self.live.rows()[index] {
            PaneRow::File { path, status } => row
                .px_2()
                .gap_3()
                .bg(theme.secondary)
                .text_color(theme.secondary_foreground)
                .child(path.display().to_string())
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(status_words(status)),
                ),
            PaneRow::Unshown { why, .. } => row
                .px_2()
                .text_color(theme.muted_foreground)
                .child(unshown_sentence(*why)),
            PaneRow::Diff { path, row: shown } => self.draw_live_diff(row, index, path, shown, cx),
            PaneRow::Thread { thread } => self.draw_live_thread(row, thread, cx),
            PaneRow::Comment { thread, comment } => self.draw_comment(row, thread, comment, "flow"),
            PaneRow::Read { path } => row
                .px_2()
                .gap_3()
                .text_color(theme.muted_foreground)
                .child(path.display().to_string())
                .child(
                    div()
                        .debug_selector(|| format!("flow-read-{index}"))
                        .flex_none()
                        .child("read"),
                ),
            PaneRow::Comparison { .. } | PaneRow::Commit { .. } | PaneRow::ThreadsElsewhere => row,
        }
    }

    fn draw_live_thread(&self, row: Div, thread: &ThreadId, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let Some(thread) = self
            .review()
            .and_then(|review| review.threads.iter().find(|held| held.id == *thread))
        else {
            return row;
        };
        let anchor = &thread.anchor;
        let side = match anchor.side {
            yaar_link::Side::Old => "old",
            yaar_link::Side::New => "new",
        };
        let place = match anchor.lines.first == anchor.lines.last {
            true => anchor.lines.last.to_string(),
            false => format!("{}-{}", anchor.lines.first, anchor.lines.last),
        };
        let id = thread.id.clone();
        row.pl_6().pr_2().text_color(theme.muted_foreground).child(
            div()
                .debug_selector(|| format!("flow-thread-{id}"))
                .child(format!(
                    "Thread on {}:{place} ({side}){}",
                    anchor.path.display(),
                    if thread.resolved { ", resolved" } else { "" }
                )),
        )
    }

    fn draw_live_diff(
        &self,
        row: Div,
        index: usize,
        path: &Path,
        shown: &Row,
        cx: &Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let Some(diff) = self.live.diff_of(path) else {
            return row;
        };
        let on = self
            .live
            .position()
            .filter(|_| self.live.position_row() == Some(index))
            .map(|position| position.side);
        let removed = theme.danger.opacity(0.16);
        let added = theme.success.opacity(0.16);
        let bar = |here: bool| {
            if here {
                theme.primary
            } else {
                transparent_black()
            }
        };
        let number = |line: Option<u32>| {
            div()
                .flex_none()
                .w(NUMBER_WIDTH)
                .px_1()
                .flex()
                .justify_end()
                .text_color(theme.muted_foreground)
                .child(line.map(|line| (line + 1).to_string()).unwrap_or_default())
        };
        let text = |side: Side, line: u32| {
            let mark = match side {
                Side::Old => theme.danger.opacity(0.4),
                Side::New => theme.success.opacity(0.4),
            };
            live_line_text(diff, index, side, line, mark)
        };
        let go = |side: Side| {
            cx.listener(move |pane, _: &MouseDownEvent, _, cx| pane.live_go_to(index, side, cx))
        };

        match (self.layout, shown) {
            (_, Row::Skipped { old, .. }) => {
                row.px_2()
                    .text_color(theme.muted_foreground)
                    .child(match old.len() {
                        1 => "⋯ 1 unchanged line".to_owned(),
                        lines => format!("⋯ {lines} unchanged lines"),
                    })
            }
            (Layout::Unified, _) => {
                let (side, sign, tint) = match shown {
                    Row::Removed { .. } => (Side::Old, "-", Some(removed)),
                    Row::Added { .. } => (Side::New, "+", Some(added)),
                    _ => (Side::New, "", None),
                };
                let Some(line) = shown.line(side) else {
                    return row;
                };
                let row = row
                    .border_l_2()
                    .border_color(bar(on.is_some()))
                    .on_mouse_down(MouseButton::Left, go(side))
                    .child(number(shown.line(Side::Old)))
                    .child(number(shown.line(Side::New)))
                    .child(div().flex_none().w(SIGN_WIDTH).px_1().child(sign))
                    .child(text(side, line));
                match tint {
                    Some(tint) => row.bg(tint),
                    None => row,
                }
            }
            (Layout::Split, _) => {
                // An unchanged line is on both sides and the reviewer is on it as a whole; a
                // changed row holds two lines, each to be on by itself.
                let unchanged = matches!(shown, Row::Context { .. });
                let half = |side: Side, tint: Hsla| {
                    let half = div()
                        .debug_selector(|| format!("flow-row-{index}-{}", side_name(side)))
                        .w_1_2()
                        .h_full()
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .border_l_2();
                    let Some(line) = shown.line(side) else {
                        return half
                            .border_color(transparent_black())
                            .bg(theme.muted.opacity(0.5));
                    };
                    let (stop, here) = if unchanged {
                        (Side::New, on.is_some())
                    } else {
                        (side, on == Some(side))
                    };
                    let half = half
                        .border_color(bar(here))
                        .on_mouse_down(MouseButton::Left, go(stop))
                        .child(number(Some(line)))
                        .child(text(side, line));
                    if unchanged { half } else { half.bg(tint) }
                };
                row.child(half(Side::Old, removed))
                    .child(half(Side::New, added))
            }
        }
    }
}

fn live_line_text(diff: &ShownDiff, index: usize, side: Side, line: u32, mark: Hsla) -> Div {
    let text = diff.text(side, line);
    // Marks count bytes of the line as in the file; where non-text bytes were replaced, nothing is
    // marked.
    let marked = match text {
        Cow::Borrowed(_) => diff.marks(side, line),
        Cow::Owned(_) => Vec::new(),
    };
    let end = text.floor_char_boundary(DRAWN_BYTES);
    let highlights = marked
        .into_iter()
        .filter(|marked| marked.start < end)
        .map(|marked| {
            let style = HighlightStyle {
                background_color: Some(mark),
                ..HighlightStyle::default()
            };
            (marked.start..marked.end.min(end), style)
        })
        .collect::<Vec<_>>();
    let drawn = text[..end].to_owned();
    div()
        .debug_selector(|| format!("flow-text-{index}-{}:{drawn}", side_name(side)))
        .flex_1()
        .min_w_0()
        .px_1()
        .overflow_hidden()
        .child(StyledText::new(drawn).with_highlights(highlights))
}
