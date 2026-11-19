//! How the live diff of Flow is drawn: one list of rows of one height in the element that
//! holds the pane's key context, as Changes is, with the line `flow-activity` above it
//! while the step Flow is at has no row in it (a command, or an edit or a write whose file
//! the list does not hold yet), and `flow-empty` while no turn is known.
//!
//! Built by the review pane's second package of slice 5, with `live.rs`. Row `i` is drawn
//! under `flow-row-<i>`, and the text of a line under `flow-text-<i>-old:<text>` or
//! `flow-text-<i>-new:<text>`, as Changes draws them under `review-`; a click on a line puts
//! the reviewer there. A file the turn read and did not change (`crate::PaneRow::Read`) is
//! drawn as its path with the mark `flow-read-<i>` that says it was read, and no hunk. A
//! thread of the review is drawn under `flow-thread-<thread id>` and each of its comments
//! under `flow-comment-<comment id>:draft` or `flow-comment-<comment id>:round-<n>`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::{Hunk, Row, Side};
use yaar_link::{Author, CommentId, FileStatus, Step, ThreadId};

use crate::draw::ROW_HEIGHT;
use crate::{Layout, PaneRow, ReviewPane, ShownDiff, Unshown};

/// The width of a column of line numbers: six digits.
const NUMBER_WIDTH: Pixels = px(56.);

/// The width of the column that holds the sign of a line in the unified view.
const SIGN_WIDTH: Pixels = px(20.);

/// How much of a line is handed to the text system, in bytes: a line is never wrapped, so
/// the rest of a very long one would be shaped for nothing, in every frame.
const DRAWN_BYTES: usize = 1024;

/// Tells the pane which rows the list laid out, once for each frame.
struct InView(Rc<RefCell<Range<usize>>>);

impl UniformListDecoration for InView {
    fn compute(
        &self,
        visible_range: Range<usize>,
        _bounds: Bounds<Pixels>,
        _scroll_offset: Point<Pixels>,
        _item_height: Pixels,
        _item_count: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        *self.0.borrow_mut() = visible_range;
        Empty.into_any_element()
    }
}

impl ReviewPane {
    /// The live diff, as the list Flow shows in its second form.
    pub(crate) fn render_live(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
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

    /// Builds the row at `index` of the rows of the live diff.
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
                .child(unshown_words(*why)),
            PaneRow::Diff { path, row: shown } => self.draw_live_diff(row, index, path, shown, cx),
            PaneRow::Thread { thread } => self.draw_live_thread(row, thread, cx),
            PaneRow::Comment { thread, comment } => self.draw_live_comment(row, thread, comment),
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
            // The live diff makes no such row.
            PaneRow::Comparison { .. } | PaneRow::Commit { .. } | PaneRow::ThreadsElsewhere => row,
        }
    }

    /// The row of a thread of the review: where it sits.
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

    /// The row of a comment of a thread: who wrote it and the first line of its text.
    fn draw_live_comment(&self, row: Div, thread: &ThreadId, comment: &CommentId) -> Div {
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
            (Author::Agent, _) => "Agent".to_owned(),
            (Author::Reviewer, Some(round)) => format!("You, round {round}"),
            (Author::Reviewer, None) => "You, draft".to_owned(),
        };
        let mut lines = comment.text.lines();
        let first = lines.next().unwrap_or_default();
        let more = if lines.next().is_some() { " …" } else { "" };
        let id = comment.id.clone();
        row.pl_10().pr_2().child(
            div()
                .debug_selector(|| format!("flow-comment-{id}:{state}"))
                .child(format!("{who}: {first}{more}")),
        )
    }

    /// Fills the row at `index` with a row of the diff of the file at `path`.
    fn draw_live_diff(
        &self,
        row: Div,
        index: usize,
        path: &Path,
        shown: &Row,
        cx: &Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        // A row of a diff is among the rows only while the pane holds the diff.
        let Some(diff) = self.live.diff_of(path) else {
            return row;
        };
        // The side the reviewer is on, when this is the row of their line.
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
                // Lines are counted from 0 and shown from 1.
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
                // An unchanged line is on both sides and the reviewer is on it as a whole;
                // a changed row holds two lines, each to be on by itself.
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

/// The name of a side in the selectors of the live diff.
fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "old",
        Side::New => "new",
    }
}

/// The ranges of bytes to mark in one line: the words that changed, when the line is part
/// of a change that both removes and adds.
fn marks(diff: &ShownDiff, side: Side, line: u32) -> Vec<Range<usize>> {
    let lines = |hunk: &Hunk| match side {
        Side::Old => hunk.old.clone(),
        Side::New => hunk.new.clone(),
    };
    let hunks = diff.diff.hunks();
    let index = hunks.partition_point(|hunk| lines(hunk).end <= line);
    if !hunks
        .get(index)
        .is_some_and(|hunk| lines(hunk).contains(&line))
    {
        return Vec::new();
    }
    let emphasis = diff
        .emphasis
        .borrow_mut()
        .entry(index)
        .or_insert_with(|| Rc::new(diff.diff.emphasis(index)))
        .clone();
    emphasis.of(side, line).to_vec()
}

/// One line of the diff shown by the row at `index`, as the element that draws it: its
/// text, with its changed words marked. The selector of the element carries the text as it
/// is handed to the text system, which a test reads.
fn live_line_text(diff: &ShownDiff, index: usize, side: Side, line: u32, mark: Hsla) -> Div {
    let text = diff.text(side, line);
    // The marks count the bytes of the line as it is in the file: where bytes that are no
    // text were replaced, the shown text has other bytes, and nothing is marked.
    let marked = match text {
        Cow::Borrowed(_) => marks(diff, side, line),
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

/// What happened to a file, beside its path.
fn status_words(status: &FileStatus) -> String {
    match status {
        FileStatus::Added => "new".to_owned(),
        FileStatus::Modified => "changed".to_owned(),
        FileStatus::Deleted => "deleted".to_owned(),
        FileStatus::Renamed { from } => format!("was {}", from.display()),
    }
}

/// What stands in the place of the lines of a file that has none to show.
fn unshown_words(why: Unshown) -> &'static str {
    match why {
        Unshown::Loading => "Reading the file",
        Unshown::Binary => "Not text: its lines are not shown",
        Unshown::TooLarge => "Too large to show",
        Unshown::Refused => "The Keeper could not read this file",
    }
}
