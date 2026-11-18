//! How the pane is drawn: one list of rows of one height, of which only those in view
//! are built.

use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::{Hunk, Row, Side};
use yaar_frame::SetBase;
use yaar_link::{Base, BaseSource, Comparison, FileStatus, NoComparison};

use crate::{KEY_CONTEXT, Layout, PaneRow, ReviewPane, ShownDiff, Unshown};

/// The height of every row. The list lays its rows out by the height of one of them, so
/// they all have this one.
pub(crate) const ROW_HEIGHT: Pixels = px(20.);

/// The width of a column of line numbers: six digits.
const NUMBER_WIDTH: Pixels = px(56.);

/// The width of the column that holds the sign of a line in the unified view.
const SIGN_WIDTH: Pixels = px(20.);

/// How much of a line is handed to the text system, in bytes. A line is never wrapped
/// and what does not fit beside the numbers is cut off, so the rest of a line of a
/// megabyte would be shaped for nothing, in every frame.
const DRAWN_BYTES: usize = 1024;

impl ShownDiff {
    /// The text of one line, as the pane shows it and as [`ReviewPane::text`] gives it.
    pub(crate) fn text(&self, side: Side, line: u32) -> Cow<'_, str> {
        String::from_utf8_lossy(self.diff.line(side, line))
    }

    /// The ranges of bytes to mark in one line: the words that changed, when the line is
    /// part of a change that both removes and adds.
    fn marks(&self, side: Side, line: u32) -> Vec<Range<usize>> {
        let lines = |hunk: &Hunk| match side {
            Side::Old => hunk.old.clone(),
            Side::New => hunk.new.clone(),
        };
        let hunks = self.diff.hunks();
        let index = hunks.partition_point(|hunk| lines(hunk).end <= line);
        let in_a_change = |hunk: &Hunk| lines(hunk).contains(&line);
        if !hunks.get(index).is_some_and(in_a_change) {
            return Vec::new();
        }
        // The words of a change are compared when it first comes into view, on the
        // thread that draws. The diff bounds what one change costs there: it compares
        // no words where there are too many.
        let emphasis = self
            .emphasis
            .borrow_mut()
            .entry(index)
            .or_insert_with(|| Rc::new(self.diff.emphasis(index)))
            .clone();
        emphasis.of(side, line).to_vec()
    }
}

/// Tells the pane which rows the list laid out. The list hands that range to what
/// decorates it, once for each frame, and to nothing else.
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

impl Render for ReviewPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let pane = div()
            .size_full()
            .flex()
            .flex_col()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::toggle_layout))
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.mono_font_family.clone())
            .text_size(theme.mono_font_size);
        if self.rows.is_empty() {
            return pane.items_center().justify_center().child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(self.idle_line(cx)),
            );
        }
        // The rows stay while the Keeper refuses to read the changes again, and may
        // then be out of date: the reviewer is told.
        let refused = self.models.changes.read(cx).refusal().map(|refusal| {
            div()
                .debug_selector(|| "review-refused".to_owned())
                .flex_none()
                .h(ROW_HEIGHT)
                .px_2()
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme.danger)
                .child(format!("The changes could not be read again: {refusal}"))
        });
        pane.children(refused).child(
            uniform_list(
                "review-rows",
                self.rows.len(),
                cx.processor(|pane, rows: Range<usize>, _, cx| {
                    rows.map(|index| pane.draw_row(index, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .with_decoration(InView(self.in_view.clone()))
            .size_full(),
        )
    }
}

impl ReviewPane {
    /// What the pane says while it has no row to show.
    fn idle_line(&self, cx: &App) -> String {
        let changes = self.models.changes.read(cx);
        match (changes.refusal(), changes.workspace()) {
            (Some(refusal), _) => refusal.to_string(),
            (None, Some(_)) => "Reading the changes".to_owned(),
            (None, None) => {
                "Open a folder that holds a git repository to see its changes".to_owned()
            }
        }
    }

    /// Builds the row at `index` of the rows of the pane.
    fn draw_row(&self, index: usize, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let row = div()
            .debug_selector(|| format!("review-row-{index}"))
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap();
        match &self.rows[index] {
            PaneRow::Comparison {
                comparison,
                offers_base,
            } => {
                // The sentence gives way where the row is too narrow for both, so the
                // offer is never pushed out of the row.
                let row = row.px_2().gap_3().bg(theme.muted).child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(comparison_sentence(comparison)),
                );
                if !offers_base {
                    return row;
                }
                // The offer runs the action of the Frame that asks for a base; the
                // Frame, which holds the pane, handles it.
                row.child(
                    div()
                        .id("review-offer-base")
                        .debug_selector(|| "review-offer-base".to_owned())
                        .flex_none()
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(theme.primary)
                        .text_color(theme.primary_foreground)
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(SetBase), cx))
                        .child("Set a base…"),
                )
            }
            PaneRow::Commit { id, summary } => row
                .px_2()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(short(id).to_owned()),
                )
                .child(summary.clone()),
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
            PaneRow::Diff { path, row: shown } => self.draw_diff(row, index, path, shown, cx),
        }
    }

    /// Fills the row at `index` with a row of the diff of the file at `path`.
    fn draw_diff(
        &self,
        row: Div,
        index: usize,
        path: &Path,
        shown: &Row,
        cx: &Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        // A row of a diff is among the rows only while the pane holds the diff.
        let Some(diff) = self.diff_of(path) else {
            return row;
        };
        // The side the reviewer is on, when this is the row of their line.
        let on = self
            .position
            .as_ref()
            .filter(|_| self.position_row == Some(index))
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
            line_text(diff, index, side, line, mark)
        };
        let go = |side: Side| {
            cx.listener(move |pane, _: &MouseDownEvent, _, cx| pane.go_to(index, side, cx))
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
                // In this layout a row shows one line. Its sign says what its colour
                // says, for a reader who cannot tell the two colours apart.
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
                // An unchanged line is on both sides and the reviewer is on it as a
                // whole; a changed row holds two lines, each to be on by itself.
                let unchanged = matches!(shown, Row::Context { .. });
                let half = |side: Side, tint: Hsla| {
                    let half = div()
                        .debug_selector(|| format!("review-row-{index}-{}", side_name(side)))
                        .w_1_2()
                        .h_full()
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .border_l_2();
                    let Some(line) = shown.line(side) else {
                        // The other side of the change is longer.
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

/// The name of a side in the selectors of the pane.
fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "old",
        Side::New => "new",
    }
}

/// One line of the diff shown by the row at `index`, as the element that draws it: its
/// text, with its changed words marked.
///
/// The selector of the element carries the text itself, as it is handed to the text
/// system here and nowhere else: a test cannot read what is drawn, and reads there
/// which text the row was given.
fn line_text(diff: &ShownDiff, index: usize, side: Side, line: u32, mark: Hsla) -> Div {
    let text = diff.text(side, line);
    // The marks count the bytes of the line as it is in the file. Where bytes that are
    // no text were replaced, the shown text has other bytes, and nothing is marked.
    let marks = match text {
        Cow::Borrowed(_) => diff.marks(side, line),
        Cow::Owned(_) => Vec::new(),
    };
    let end = text.floor_char_boundary(DRAWN_BYTES);
    let highlights = marks
        .into_iter()
        .filter(|mark| mark.start < end)
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
        .debug_selector(|| format!("review-text-{index}-{}:{drawn}", side_name(side)))
        .flex_1()
        .min_w_0()
        .px_1()
        .overflow_hidden()
        .child(StyledText::new(drawn).with_highlights(highlights))
}

/// The first seven digits of the id of a commit, as git shortens it.
fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// A base as a reader names it: its branch, or its commit shortened, and who chose it.
fn base_words(base: &Base) -> String {
    // A base that is not a branch is kept as the full id of a commit.
    let is_commit = matches!(base.name.len(), 40 | 64)
        && base.name.bytes().all(|byte| byte.is_ascii_hexdigit());
    let name = if is_commit {
        short(&base.name)
    } else {
        &base.name
    };
    match base.source {
        BaseSource::Default => format!("{name}, the default branch"),
        BaseSource::User => format!("{name}, the base you set"),
    }
}

/// What the first row says: what the changes are compared with, or why with nothing.
fn comparison_sentence(comparison: &Comparison) -> String {
    const ONLY_UNCOMMITTED: &str =
        "nothing to compare with, so only the uncommitted changes are shown";
    match comparison {
        Comparison::SinceBase { base } => format!(
            "Compared with {}: the changes since the branch left it",
            base_words(base)
        ),
        Comparison::NotOnUpstream { base, upstream } => format!(
            "On the base branch ({}): the commits that are not on {upstream}, and the uncommitted changes",
            base_words(base)
        ),
        Comparison::OnNoRemoteBranch { base } => format!(
            "On the base branch ({}), which follows no upstream: the commits on no branch of a remote, and the uncommitted changes",
            base_words(base)
        ),
        Comparison::NothingToCompare { base, why } => match (why, base) {
            (NoComparison::NoRemote, _) => {
                format!("On the base branch of a repository with no remote: {ONLY_UNCOMMITTED}")
            }
            (NoComparison::BaseGone, Some(base)) => format!(
                "Git no longer knows {}: {ONLY_UNCOMMITTED}",
                base_words(base)
            ),
            (NoComparison::NoSharedCommit, Some(base)) => format!(
                "The branch shares no commit with {}: {ONLY_UNCOMMITTED}",
                base_words(base)
            ),
            (NoComparison::NoBase | NoComparison::BaseGone | NoComparison::NoSharedCommit, _) => {
                format!("This workspace has no base: {ONLY_UNCOMMITTED}")
            }
        },
    }
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
fn unshown_sentence(why: Unshown) -> &'static str {
    match why {
        Unshown::Loading => "Reading the file",
        Unshown::Binary => "Not text: its lines are not shown",
        Unshown::TooLarge => "Too large to show",
        Unshown::Refused => "The Keeper could not read this file",
    }
}
