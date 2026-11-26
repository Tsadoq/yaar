//! How the live diff of Flow is drawn: one list of rows using the shared diff styling.

use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yaar_diff::{Row, Side};
use yaar_link::{Step, ThreadId};
use yaar_ui::diff::{DiffKind, DiffStats, bar, gap_fill, ground, sign_colour, word};
use yaar_ui::metrics::{SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, tabular};
use yaar_ui::pieces::{
    Banner, Button, ButtonKind, Chip, Divider, EmptyState, State, StateDot, Tone,
};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::draw::{
    DRAWN_BYTES, InView, NUMBER_WIDTH, ROW_HEIGHT, SIGN_WIDTH, diff_highlights, side_name,
    status_words, unshown_sentence,
};
use crate::flow_draw::format_line_ranges;
use crate::{Layout, PaneRow, ReviewPane, ShownDiff};

impl ReviewPane {
    pub(crate) fn render_live(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let activity = self.activity().map(|at| {
            let words = match self.flow.step(at) {
                Some(Step::Command { command }) => {
                    format!("Run {}", command.lines().next().unwrap_or_default())
                }
                Some(Step::Edit { path, lines }) => {
                    let spans = format_line_ranges(lines.as_deref());
                    if spans.is_empty() {
                        format!("Editing {}", path.display())
                    } else {
                        format!("Editing {} lines {spans}", path.display())
                    }
                }
                Some(Step::Write { path }) => format!("Write {}", path.display()),
                Some(Step::Read { path, lines }) => {
                    let spans = lines
                        .as_ref()
                        .map(|r| format_line_ranges(Some(std::slice::from_ref(r))))
                        .unwrap_or_default();
                    if spans.is_empty() {
                        format!("Read {}", path.display())
                    } else {
                        format!("Read {} lines {spans}", path.display())
                    }
                }
                None => "The newest step".to_owned(),
            };
            Banner::new(words).live().selector("flow-activity")
        });

        if self.live.reading().is_some() {
            let banner = self
                .activity()
                .filter(|at| !matches!(self.flow.step(at), Some(Step::Read { .. })))
                .and(activity);
            return self.render_live_read(banner, cx);
        }

        if self.live.change().is_none() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("No turn of the agent to show yet")
                        .line("Live diff follows the agent's reads and edits as they land.")
                        .selector("flow-empty"),
                )
                .into_any_element();
        }
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

    fn render_live_read(&mut self, activity: Option<Banner>, cx: &mut Context<Self>) -> AnyElement {
        let palette = palette(cx);
        let Some(reading) = self.live.reading() else {
            return div().into_any_element();
        };
        let path_text = reading.path.display().to_string();
        let line_chip = reading.lines.as_ref().map(|range| {
            let spans = format_line_ranges(Some(std::slice::from_ref(range)));
            Chip::new(format!("lines {spans}")).tone(Tone::Info)
        });
        let running = self
            .live
            .change()
            .is_none_or(|change| self.flow.is_running(&change.turn));
        let edited_count = self.live.files_count();
        let back_to_diff = (edited_count > 0).then(|| {
            Button::new(
                "flow-live-back-to-diff",
                format!("Live diff ({edited_count})"),
            )
            .kind(ButtonKind::Ghost)
            .small()
            .selector("flow-live-back-to-diff")
            .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                pane.steered(cx);
                pane.live_clear_read(cx);
            }))
        });

        let header = div()
            .debug_selector(|| "flow-live-read-header".to_owned())
            .flex_none()
            .w_full()
            .h(px(34.))
            .px(SPACE_5)
            .bg(palette.raised)
            .border_b_1()
            .border_color(palette.separator)
            .flex()
            .items_center()
            .gap(SPACE_3)
            .overflow_hidden()
            .whitespace_nowrap()
            .child(StateDot::new(if running {
                State::Working
            } else {
                State::Idle
            }))
            .child(
                div()
                    .flex_none()
                    .typed(TypeRole::Small)
                    .text_color(palette.text_muted)
                    .child("Reading"),
            )
            .child(
                div()
                    .debug_selector({
                        let path_text = path_text.clone();
                        move || format!("flow-live-read-path:{path_text}")
                    })
                    .min_w_0()
                    .truncate()
                    .typed(TypeRole::Path)
                    .text_color(palette.text)
                    .child(path_text),
            )
            .children(line_chip)
            .child(div().flex_1())
            .children(back_to_diff);

        let line_count = reading.line_ranges.len();
        let scroll = reading.scroll.clone();
        let in_view = reading.in_view.clone();
        let list = uniform_list(
            "flow-live-read",
            line_count,
            cx.processor(|pane, rows: Range<usize>, _, cx| {
                rows.map(|index| pane.draw_live_read_line(index, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&scroll)
        .with_decoration(InView(in_view))
        .size_full();

        div()
            .debug_selector(|| "flow-live-read-view".to_owned())
            .size_full()
            .flex()
            .flex_col()
            .children(activity)
            .child(header)
            .child(div().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    fn draw_live_read_line(&self, index: usize, cx: &Context<Self>) -> Div {
        let palette = palette(cx);
        let Some(reading) = self.live.reading() else {
            return div().h(ROW_HEIGHT).w_full();
        };
        let line = index as u32 + 1;
        let in_range = reading
            .lines
            .as_ref()
            .is_some_and(|range| range.contains(&line));
        let text = reading
            .line_ranges
            .get(index)
            .and_then(|range| reading.opened.text.get(range.clone()))
            .unwrap_or_default();
        let end = text.floor_char_boundary(DRAWN_BYTES);
        let drawn = text[..end].to_owned();
        let highlights = diff_highlights(&reading.path, &drawn, &[], None, &palette);
        div()
            .debug_selector(move || {
                if in_range {
                    format!("flow-live-read-active-{line}")
                } else {
                    format!("flow-live-read-line-{line}")
                }
            })
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_l_2()
            .border_color(if in_range {
                palette.focus
            } else {
                transparent_black()
            })
            .when(in_range, |row| row.bg(palette.raised))
            .child(
                div()
                    .flex_none()
                    .w(NUMBER_WIDTH)
                    .pr(SPACE_4)
                    .flex()
                    .justify_end()
                    .typed(TypeRole::CodeSmall)
                    .font_features(tabular())
                    .text_color(if in_range {
                        palette.text
                    } else {
                        palette.text_dim
                    })
                    .child(line.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px(SPACE_2)
                    .overflow_hidden()
                    .typed(TypeRole::Code)
                    .text_color(palette.text)
                    .child(StyledText::new(drawn).with_highlights(highlights)),
            )
    }

    fn draw_live_row(&self, index: usize, cx: &Context<Self>) -> Div {
        let palette = palette(cx);
        let row = div()
            .debug_selector(|| format!("flow-row-{index}"))
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap();
        match &self.live.rows()[index] {
            PaneRow::File { path, status } => {
                let stats = self.live.diff_of(path).map(|shown| {
                    let added: usize = shown.diff.hunks().iter().map(|hunk| hunk.new.len()).sum();
                    let removed: usize = shown.diff.hunks().iter().map(|hunk| hunk.old.len()).sum();
                    DiffStats::new(added, removed).small()
                });
                row.px(SPACE_6)
                    .gap(SPACE_4)
                    .bg(palette.raised)
                    .border_t_1()
                    .border_b_1()
                    .border_color(palette.separator)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .typed(TypeRole::Path)
                            .text_color(palette.text)
                            .child(path.display().to_string()),
                    )
                    .child(Chip::new(status_words(status)))
                    .children(stats)
            }
            PaneRow::Unshown { why, .. } => row
                .px(SPACE_6)
                .typed(TypeRole::Meta)
                .text_color(palette.text_muted)
                .child(unshown_sentence(*why)),
            PaneRow::Diff { path, row: shown } => self.draw_live_diff(row, index, path, shown, cx),
            PaneRow::Thread { thread } => self.draw_live_thread(row, thread, cx),
            PaneRow::Comment { thread, comment } => {
                self.draw_live_comment(row, thread, comment, cx)
            }
            PaneRow::Read { path } => {
                let target = path.clone();
                row.px(SPACE_6)
                    .gap(SPACE_4)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                            pane.live_open_read(target.clone(), None, cx);
                        }),
                    )
                    .child(
                        div()
                            .typed(TypeRole::Path)
                            .text_color(palette.text_muted)
                            .child(path.display().to_string()),
                    )
                    .child(Chip::new("read").selector(format!("flow-read-{index}")))
            }
            PaneRow::Commit { .. } | PaneRow::ThreadsElsewhere => row,
        }
    }

    fn draw_live_thread(&self, row: Div, thread: &ThreadId, cx: &Context<Self>) -> Div {
        let palette = palette(cx);
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
        row.pl(px(60.))
            .pr(SPACE_6)
            .bg(palette.window)
            .typed(TypeRole::Meta)
            .text_color(palette.link)
            .child(
                div()
                    .debug_selector(|| format!("flow-thread-{id}"))
                    .child(format!(
                        "Thread on {}:{place} ({side}){}",
                        anchor.path.display(),
                        if thread.resolved { ", resolved" } else { "" }
                    )),
            )
    }

    fn draw_live_comment(
        &self,
        row: Div,
        thread: &ThreadId,
        comment: &yaar_link::CommentId,
        cx: &Context<Self>,
    ) -> Div {
        let palette = palette(cx);
        let Some(comment) = self
            .review()
            .and_then(|review| review.threads.iter().find(|held| held.id == *thread))
            .and_then(|thread| thread.comments.iter().find(|held| held.id == *comment))
        else {
            return row;
        };
        let state = match comment.round {
            Some(round) => format!("round-{round}"),
            None => "draft".to_owned(),
        };
        let who = match (comment.author, comment.round) {
            (yaar_link::Author::Agent, _) => "Agent".to_owned(),
            (yaar_link::Author::Reviewer, Some(round)) => format!("You, round {round}"),
            (yaar_link::Author::Reviewer, None) => "You, draft".to_owned(),
        };
        let mut lines = comment.text.lines();
        let first = lines.next().unwrap_or_default();
        let more = if lines.next().is_some() { " …" } else { "" };
        let id = comment.id.clone();
        row.pl(px(72.))
            .pr(SPACE_6)
            .bg(palette.window)
            .typed(TypeRole::Body)
            .text_color(palette.text)
            .child(
                div()
                    .debug_selector(|| format!("flow-comment-{id}:{state}"))
                    .child(format!("{who}: {first}{more}")),
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
        let palette = palette(cx);
        let Some(diff) = self.live.diff_of(path) else {
            return row;
        };
        let on = self
            .live
            .position()
            .filter(|_| self.live.position_row() == Some(index))
            .map(|position| position.side);
        let focus_bar = |here: bool| {
            if here {
                palette.focus
            } else {
                transparent_black()
            }
        };
        let number = |line: Option<u32>| {
            div()
                .flex_none()
                .w(NUMBER_WIDTH)
                .pr(SPACE_4)
                .flex()
                .justify_end()
                .typed(TypeRole::CodeSmall)
                .font_features(tabular())
                .text_color(palette.text_dim)
                .child(line.map(|line| (line + 1).to_string()).unwrap_or_default())
        };
        let text = |side: Side, line: u32, kind: DiffKind| {
            live_line_text(diff, path, index, side, line, kind, &palette)
        };
        let go = |side: Side| {
            cx.listener(move |pane, _: &MouseDownEvent, _, cx| pane.live_go_to(index, side, cx))
        };

        match (self.layout, shown) {
            (_, Row::Skipped { old, .. }) => row
                .pl(px(98.))
                .pr(SPACE_6)
                .bg(palette.fold_ground)
                .typed(TypeRole::Meta)
                .text_color(palette.text_dim)
                .child(match old.len() {
                    1 => "⋯ 1 unchanged line".to_owned(),
                    lines => format!("⋯ {lines} unchanged lines"),
                }),
            (Layout::Unified, _) => {
                let (side, sign, kind) = match shown {
                    Row::Removed { .. } => (Side::Old, "-", DiffKind::Removed),
                    Row::Added { .. } => (Side::New, "+", DiffKind::Added),
                    _ => (Side::New, "", DiffKind::Context),
                };
                let Some(line) = shown.line(side) else {
                    return row;
                };
                let row = row
                    .border_l_2()
                    .border_color(focus_bar(on.is_some()))
                    .on_mouse_down(MouseButton::Left, go(side))
                    .child(number(shown.line(Side::Old)))
                    .child(number(shown.line(Side::New)))
                    .child(bar(kind, &palette))
                    .child(
                        div()
                            .flex_none()
                            .w(SIGN_WIDTH)
                            .flex()
                            .justify_center()
                            .typed(TypeRole::Code)
                            .text_color(sign_colour(kind, &palette))
                            .child(sign),
                    )
                    .child(text(side, line, kind));
                match ground(kind, &palette) {
                    Some(tint) => row.bg(tint),
                    None => row,
                }
            }
            (Layout::Split, _) => {
                let unchanged = matches!(shown, Row::Context { .. });
                let half = |side: Side, kind: DiffKind| {
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
                            .child(gap_fill(&palette));
                    };
                    let (stop, here) = if unchanged {
                        (Side::New, on.is_some())
                    } else {
                        (side, on == Some(side))
                    };
                    let line_kind = if unchanged { DiffKind::Context } else { kind };
                    let half = half
                        .border_color(focus_bar(here))
                        .on_mouse_down(MouseButton::Left, go(stop))
                        .child(number(Some(line)))
                        .child(bar(line_kind, &palette))
                        .child(text(side, line, line_kind));
                    match ground(line_kind, &palette) {
                        Some(tint) => half.bg(tint),
                        None => half,
                    }
                };
                row.child(half(Side::Old, DiffKind::Removed))
                    .child(Divider::vertical())
                    .child(half(Side::New, DiffKind::Added))
            }
        }
    }
}

fn live_line_text(
    diff: &ShownDiff,
    path: &Path,
    index: usize,
    side: Side,
    line: u32,
    kind: DiffKind,
    palette: &yaar_ui::tokens::Palette,
) -> Div {
    let text = diff.text(side, line);
    let marked = match text {
        Cow::Borrowed(_) => diff.marks(side, line),
        Cow::Owned(_) => Vec::new(),
    };
    let end = text.floor_char_boundary(DRAWN_BYTES);
    let drawn = text[..end].to_owned();
    let highlights = diff_highlights(path, &drawn, &marked, word(kind, palette), palette);
    div()
        .debug_selector(|| format!("flow-text-{index}-{}:{drawn}", side_name(side)))
        .flex_1()
        .min_w_0()
        .px(SPACE_2)
        .overflow_hidden()
        .typed(TypeRole::Code)
        .text_color(palette.text)
        .child(StyledText::new(drawn).with_highlights(highlights))
}
