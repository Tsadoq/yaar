//! How the review pane is drawn: UI chrome uses `yaar-ui` pieces and Inter; code and paths use
//! JetBrains Mono with syntax and word-diff highlights.

use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use yaar_diff::{Hunk, Row, Side};
use yaar_link::{
    Author, Base, BaseSource, CommentId, Comparison, FileStatus, NoComparison, ThreadId,
    ThreadState,
};
use yaar_ui::diff::{
    DiffKind, DiffStats, GutterPlus, ViewedToggle, bar, gap_fill, ground, sign_colour, word,
};
use yaar_ui::metrics::{SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, tabular};
use yaar_ui::pieces::{
    Avatar, Banner, Button, ButtonKind, Card, CardPart, Chip, Divider, EmptyState, Field, Tone,
};
use yaar_ui::syntax;
use yaar_ui::themes::palette;
use yaar_ui::tokens::Palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::navigate::Spot;
use crate::{ChangeShown, KEY_CONTEXT, Layout, PaneRow, ReviewPane, ShownDiff, Unshown, View};

/// Height of one code diff row, also used as the uniform height hint for unmeasured list items.
pub(crate) const ROW_HEIGHT: Pixels = px(20.);
pub(crate) const COMMIT_ROW_HEIGHT: Pixels = px(26.);
pub(crate) const UNSHOWN_ROW_HEIGHT: Pixels = px(28.);
pub(crate) const FILE_HEADER_HEIGHT: Pixels = px(34.);

const GUTTER_WIDTH: Pixels = px(22.);
pub(crate) const NUMBER_WIDTH: Pixels = px(38.);
pub(crate) const SIGN_WIDTH: Pixels = px(14.);

/// Longest line handed to the text system. A whole line is kept in memory for comments and
/// diffs; drawing more than this on one row lays out glyphs the screen cannot show.
pub(crate) const DRAWN_BYTES: usize = 1024;

impl ShownDiff {
    pub(crate) fn text(&self, side: Side, line: u32) -> Cow<'_, str> {
        String::from_utf8_lossy(self.diff.line(side, line))
    }

    pub(crate) fn marks(&self, side: Side, line: u32) -> Vec<Range<usize>> {
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
        // Words of a change are compared when it first comes into view, on the drawing thread.
        let emphasis = self
            .emphasis
            .borrow_mut()
            .entry(index)
            .or_insert_with(|| Rc::new(self.diff.emphasis(index)))
            .clone();
        emphasis.of(side, line).to_vec()
    }
}

/// Records the rows a uniform list paints (used by Flow's timeline and live diff).
pub(crate) struct InView(pub(crate) Rc<RefCell<Range<usize>>>);

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_cursor();
        let palette = palette(cx);
        let left_column = self.render_left_column(window, cx);
        let has_left = left_column.is_some();
        let rounds = (!has_left).then(|| self.render_rounds(cx)).flatten();
        let start = self.render_offer_to_start(cx);
        let offer = self.offer_bar(cx);
        let bar = self.render_bar(cx);
        let servers = self.render_servers(cx);
        let hover = self.render_hover(window, cx);
        let references = self.render_references(window, cx);
        let file_view = self.render_file_view(cx);
        let flow = (self.view == View::Flow).then(|| self.render_flow(cx));
        let pane = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.pane)
            .typed(TypeRole::Body)
            .text_color(palette.text);
        // Anything inside `body` is what the reviewer reads: acting on it pauses follow.
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .capture_any_mouse_down(cx.listener(|pane, _: &MouseDownEvent, _, cx| pane.touched(cx)))
            .capture_key_down(cx.listener(|pane, _: &KeyDownEvent, _, cx| pane.touched(cx)))
            .on_scroll(cx.listener(|pane, _: &ScrollEvent, _, cx| {
                pane.scrolled();
                pane.touched(cx);
            }));
        let notice = self.notice.as_ref().map(|words| {
            div()
                .flex_none()
                .border_t_1()
                .border_color(palette.separator)
                .child(
                    Banner::new(words.clone())
                        .tone(Tone::Danger)
                        .selector("review-notice"),
                )
        });
        let comment_box = self.open_box.as_ref().map(|open| {
            let header = div()
                .debug_selector(|| "review-comment-box-header".to_owned())
                .typed(TypeRole::CodeSmall)
                .text_color(palette.text_muted)
                .child(format!(
                    "Comment on {}:{} ({})",
                    open.view.at.path.display(),
                    open.view.at.line + 1,
                    side_name(open.view.at.side),
                ));
            let footer = div()
                .w_full()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .flex_1()
                        .typed(TypeRole::Tiny)
                        .text_color(palette.text_dim)
                        .child("Draft saves as you type · Enter closes · Shift+Enter newline"),
                )
                .child(
                    Button::new("review-comment-done", "Done")
                        .kind(ButtonKind::Primary)
                        .small()
                        .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                            pane.close_box(window, cx);
                        })),
                );
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .px(SPACE_6)
                .py(SPACE_4)
                .border_t_1()
                .border_color(palette.separator)
                .on_action(cx.listener(Self::escape))
                .child(header)
                .child(
                    Field::textarea(&open.input)
                        .footer(footer)
                        .selector("review-comment-box"),
                )
        });
        let commit_field = self.commit_field.as_ref().map(|field| {
            let footer = div()
                .w_full()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .flex_1()
                        .typed(TypeRole::Tiny)
                        .text_color(palette.text_dim)
                        .child("Enter commits · Esc cancels"),
                )
                .child(
                    Button::new("review-commit-cancel", "Cancel")
                        .kind(ButtonKind::Ghost)
                        .small()
                        .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                            pane.close_commit_field(window, cx);
                        })),
                );
            div()
                .flex_none()
                .px(SPACE_6)
                .py(SPACE_4)
                .border_t_1()
                .border_color(palette.separator)
                .on_action(cx.listener(Self::escape_commit))
                .child(
                    Field::input(&field.input)
                        .footer(footer)
                        .selector("review-commit-field"),
                )
        });
        if let Some(file_view) = file_view {
            return pane.children(bar).children(servers).child(
                body.child(self.with_file_keys(div().flex_1().min_h_0().child(file_view), cx))
                    .children(hover)
                    .children(references)
                    .children(notice)
                    .children(comment_box),
            );
        }
        if let Some(flow) = flow {
            return pane.children(bar).children(servers).child(
                body.child(self.with_keys(div().flex_1().min_h_0().child(flow), cx))
                    .children(hover)
                    .children(references)
                    .children(notice)
                    .children(comment_box)
                    .children(commit_field),
            );
        }
        if self.rows.is_empty() {
            let (title, detail) = self.idle_lines(cx);
            let mut empty = EmptyState::new(title);
            if let Some(detail) = detail {
                empty = empty.line(detail);
            }
            let idle = div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .child(empty);
            let main_area = div()
                .flex_1()
                .min_h_0()
                .flex()
                .children(left_column)
                .child(self.with_keys(idle, cx));
            return pane.children(bar).children(servers).child(
                body.children(start)
                    .children(rounds)
                    .children(offer)
                    .child(main_area)
                    .children(notice)
                    .children(comment_box)
                    .children(commit_field),
            );
        }
        if !self.first_diff_marked
            && self
                .rows
                .iter()
                .any(|row| matches!(row, PaneRow::Diff { .. }))
        {
            self.first_diff_marked = true;
            window.on_next_frame(|_, _| yaar_frame::trace::mark_once("first_diff"));
        }
        let refused = self.models.changes.read(cx).refusal().map(|refusal| {
            Banner::new(format!("The changes could not be read again: {refusal}"))
                .tone(Tone::Danger)
                .selector("review-refused")
        });
        let list_view = list(
            self.list.clone(),
            cx.processor(|pane, index: usize, window, cx| pane.draw_row(index, window, cx)),
        )
        .size_full();
        let current_width = self.list.viewport_bounds().size.width;
        if current_width != self.list_width || (self.list_width == px(0.) && !self.rows.is_empty())
        {
            cx.defer_in(window, |pane, _, cx| {
                let width = pane.list.viewport_bounds().size.width;
                if width > px(0.) && width != pane.list_width {
                    pane.list_width = width;
                    pane.h_scroll = pane.h_scroll.min(pane.max_h_scroll());
                    if !pane.rows.is_empty() {
                        let top = pane.list.logical_scroll_top();
                        pane.sync_list(Some(top));
                        cx.notify();
                    }
                }
            });
        }
        let main_area = div().flex_1().min_h_0().flex().children(left_column).child(
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .children(refused)
                .child(
                    self.with_keys(
                        div()
                            .flex_1()
                            .min_h_0()
                            .on_scroll(cx.listener(|pane, event: &ScrollEvent, _, cx| {
                                pane.scroll_horizontal(event, cx);
                            }))
                            .child(list_view),
                        cx,
                    ),
                ),
        );
        pane.children(bar).children(servers).child(
            body.children(start)
                .children(rounds)
                .children(offer)
                .child(main_area)
                .children(hover)
                .children(references)
                .children(notice)
                .children(comment_box)
                .children(commit_field),
        )
    }
}

impl ReviewPane {
    fn with_keys(&self, element: Div, cx: &Context<Self>) -> Div {
        element
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::focus_history))
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::toggle_layout))
            .on_action(cx.listener(Self::comment))
            .on_action(cx.listener(Self::edit_draft))
            .on_action(cx.listener(Self::reply))
            .on_action(cx.listener(Self::delete_draft))
            .on_action(cx.listener(Self::next_thread))
            .on_action(cx.listener(Self::previous_thread))
            .on_action(cx.listener(Self::copy_review))
            .on_action(cx.listener(Self::toggle_viewed))
            .on_action(cx.listener(Self::mark_reviewed))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::toggle_keep_selection))
            .on_action(cx.listener(Self::take_offer))
            .on_action(cx.listener(Self::next_review))
            .on_action(cx.listener(Self::toggle_view))
            .on_action(cx.listener(Self::toggle_flow_form_in_flow))
            .on_action(cx.listener(Self::toggle_follow))
            .on_action(cx.listener(Self::resume_follow_in_flow))
            .on_action(cx.listener(Self::attach_review))
            .on_action(cx.listener(Self::attach_thread))
            .on_action(cx.listener(Self::detach_review))
            .on_action(cx.listener(Self::resolve_thread))
            .on_action(cx.listener(Self::reopen_thread))
            .on_action(cx.listener(Self::resolve_addressed))
            .on_action(cx.listener(Self::show_original))
            .on_action(cx.listener(Self::since_round))
            .on_action(cx.listener(Self::start_agent))
            .on_action(cx.listener(Self::next_offered_agent))
            .on_action(cx.listener(Self::next_name))
            .on_action(cx.listener(Self::previous_name))
            .on_action(cx.listener(Self::hover_name))
            .on_action(cx.listener(Self::go_to_definition))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::download_server))
            .on_action(cx.listener(Self::update_server))
            .on_action(cx.listener(Self::allow_servers))
            .on_action(cx.listener(Self::refuse_servers))
    }

    fn with_file_keys(&self, element: Div, cx: &Context<Self>) -> Div {
        element
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::comment))
            .on_action(cx.listener(Self::next_name))
            .on_action(cx.listener(Self::previous_name))
            .on_action(cx.listener(Self::hover_name))
            .on_action(cx.listener(Self::go_to_definition))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::download_server))
            .on_action(cx.listener(Self::update_server))
            .on_action(cx.listener(Self::allow_servers))
            .on_action(cx.listener(Self::refuse_servers))
    }

    fn toggle_flow_form_in_flow(
        &mut self,
        _: &crate::ToggleFlowForm,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::Flow {
            self.toggle_flow_form(cx);
        }
    }

    fn resume_follow_in_flow(
        &mut self,
        _: &crate::ResumeFollow,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::Flow {
            self.resume_follow(cx);
        }
    }

    fn idle_lines(&self, cx: &App) -> (String, Option<String>) {
        let changes = self.models.changes.read(cx);
        match (changes.refusal(), changes.workspace(), &self.changes) {
            (Some(refusal), _, _) => (
                "Could not read changes".to_owned(),
                Some(refusal.to_string()),
            ),
            (None, Some(_), Some(_)) => (
                "No changes to review".to_owned(),
                Some("Edits and commits in this workspace will appear here.".to_owned()),
            ),
            (None, Some(_), None) => ("Reading the changes".to_owned(), None),
            (None, None, _) => (
                "Open a folder that holds a git repository".to_owned(),
                Some("Changes, history, and review threads will appear here.".to_owned()),
            ),
        }
    }

    fn draw_row(&self, index: usize, window: &Window, cx: &Context<Self>) -> AnyElement {
        let palette = palette(cx);
        let focused = self.focus.is_focused(window);
        match &self.rows[index] {
            PaneRow::Commit { id, summary } => div()
                .debug_selector(|| format!("review-row-{index}"))
                .h(COMMIT_ROW_HEIGHT)
                .w_full()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .px(SPACE_5)
                .overflow_hidden()
                .whitespace_nowrap()
                .bg(palette.fold_ground)
                .border_b_1()
                .border_color(palette.separator)
                .child(
                    div()
                        .flex_none()
                        .typed(TypeRole::CodeSmall)
                        .text_color(palette.text_dim)
                        .child(short(id).to_owned()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .typed(TypeRole::Meta)
                        .text_color(palette.text_muted)
                        .child(summary.clone()),
                )
                .into_any_element(),
            PaneRow::File { path, status } => {
                let viewed = self
                    .reviews()
                    .is_some_and(|reviews| reviews.viewed.contains(path));
                let no_agent =
                    self.tree.shown == ChangeShown::Uncommitted && self.made_by_no_agent(path);
                let shared = self.shared_mark(index, path, cx);
                let stats = self.diff_of(path).map(|shown| {
                    let added: usize = shown.diff.hunks().iter().map(|hunk| hunk.new.len()).sum();
                    let removed: usize = shown.diff.hunks().iter().map(|hunk| hunk.old.len()).sum();
                    DiffStats::new(added, removed)
                });
                let file_path = path.clone();
                let mut header = div()
                    .debug_selector(|| format!("review-row-{index}"))
                    .min_h(FILE_HEADER_HEIGHT)
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .px(SPACE_6)
                    .py(SPACE_2)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .bg(palette.fold_ground)
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
                    .children(stats);
                if !matches!(status, FileStatus::Modified) {
                    header = header.child(Chip::new(status_words(status)));
                }
                if no_agent {
                    header = header.child(
                        Chip::new("no agent")
                            .tone(Tone::Warn)
                            .selector(format!("review-no-agent-{index}")),
                    );
                }
                header = header.children(shared).child(div().flex_1());
                let mut viewed_box = ViewedToggle::new(("review-viewed", index), viewed)
                    .tooltip("Mark file as viewed (r)")
                    .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                        pane.toggle_viewed_path(file_path.clone(), cx);
                    }));
                if viewed {
                    viewed_box = viewed_box.selector(format!("review-viewed-{index}"));
                }
                header.child(viewed_box).into_any_element()
            }
            PaneRow::Unshown { why, .. } => div()
                .debug_selector(|| format!("review-row-{index}"))
                .h(UNSHOWN_ROW_HEIGHT)
                .w_full()
                .flex()
                .items_center()
                .px(SPACE_6)
                .typed(TypeRole::Meta)
                .text_color(palette.text_muted)
                .child(unshown_sentence(*why))
                .into_any_element(),
            PaneRow::Diff { path, row: shown } => {
                let row = div()
                    .debug_selector(|| format!("review-row-{index}"))
                    .h(ROW_HEIGHT)
                    .w_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap();
                self.draw_diff(row, index, path, shown, focused, cx)
                    .into_any_element()
            }
            PaneRow::Thread { thread } => self.draw_thread_card(index, thread, cx),
            PaneRow::Comment { thread, comment } => {
                self.draw_comment_card(index, thread, comment, cx)
            }
            PaneRow::ThreadsElsewhere => div()
                .debug_selector(|| format!("review-row-{index}"))
                .w_full()
                .px(SPACE_6)
                .pt(SPACE_5)
                .pb(SPACE_3)
                .bg(palette.window)
                .border_t_1()
                .border_b_1()
                .border_color(palette.separator)
                .child(
                    div()
                        .debug_selector(|| "review-threads-elsewhere".to_owned())
                        .typed(TypeRole::MetaStrong)
                        .text_color(palette.text_muted)
                        .child("Threads elsewhere: their lines are in no row shown"),
                )
                .into_any_element(),
            PaneRow::Read { path } => div()
                .debug_selector(|| format!("review-row-{index}"))
                .h(ROW_HEIGHT)
                .w_full()
                .flex()
                .items_center()
                .px(SPACE_6)
                .typed(TypeRole::Path)
                .text_color(palette.text_muted)
                .child(path.display().to_string())
                .into_any_element(),
        }
    }

    fn thread_indent(&self) -> Pixels {
        match self.layout {
            Layout::Unified => px(100.),
            Layout::Split => SPACE_6,
        }
    }

    fn draw_thread_card(
        &self,
        index: usize,
        thread_id: &ThreadId,
        cx: &Context<Self>,
    ) -> AnyElement {
        let palette = palette(cx);
        let Some(thread) = self
            .review()
            .and_then(|review| review.threads.iter().find(|held| held.id == *thread_id))
        else {
            return div()
                .debug_selector(|| format!("review-row-{index}"))
                .into_any_element();
        };
        let anchor = &thread.anchor;
        let side = match anchor.side {
            yaar_link::Side::Old => "old",
            yaar_link::Side::New => "new",
        };
        let place = if anchor.lines.first == anchor.lines.last {
            anchor.lines.last.to_string()
        } else {
            format!("{}-{}", anchor.lines.first, anchor.lines.last)
        };
        let id = thread.id.clone();
        let resolved = thread.resolved;
        let has_comments = matches!(
            self.rows.get(index + 1),
            Some(PaneRow::Comment { thread: next, .. }) if next == &thread.id
        );
        let state = self.thread_state_mark(&thread.id, cx);
        let moved = self.thread_moved_marks(&thread.id, cx);
        let original = self.thread_original_block(&thread.id, cx);

        let resolve_id = thread.id.clone();
        let resolve_button = Button::new(
            SharedString::from(format!("review-thread-resolve-{id}")),
            if resolved { "Reopen" } else { "Resolve" },
        )
        .kind(ButtonKind::Ghost)
        .small()
        .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
            pane.resolve_or_reopen_thread(resolve_id.clone(), !resolved, cx);
        }));

        let header = div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(SPACE_3)
            .child(
                div()
                    .debug_selector(|| format!("review-thread-{id}"))
                    .typed(TypeRole::CodeSmall)
                    .text_color(palette.link)
                    .child(format!(
                        "{}:{place} ({side}){}",
                        anchor.path.display(),
                        if resolved { " · resolved" } else { "" }
                    )),
            )
            .children(state)
            .children(moved)
            .child(div().flex_1())
            .child(resolve_button);

        let card = Card::new()
            .elevated()
            .accent(thread_accent(thread.state, &palette))
            .part(if has_comments {
                CardPart::Top
            } else {
                CardPart::Whole
            })
            .max_w(px(760.))
            .header(header)
            .children(original);

        div()
            .debug_selector(|| format!("review-row-{index}"))
            .w_full()
            .pl(self.thread_indent())
            .pr(SPACE_6)
            .pt(SPACE_4)
            .when(!has_comments, |el| el.pb(SPACE_4))
            .child(card)
            .into_any_element()
    }

    fn draw_comment_card(
        &self,
        index: usize,
        thread_id: &ThreadId,
        comment_id: &CommentId,
        cx: &Context<Self>,
    ) -> AnyElement {
        let palette = palette(cx);
        let Some(thread) = self
            .review()
            .and_then(|review| review.threads.iter().find(|held| held.id == *thread_id))
        else {
            return div()
                .debug_selector(|| format!("review-row-{index}"))
                .into_any_element();
        };
        let Some(comment) = thread.comments.iter().find(|held| held.id == *comment_id) else {
            return div()
                .debug_selector(|| format!("review-row-{index}"))
                .into_any_element();
        };
        let is_first = thread
            .comments
            .first()
            .is_some_and(|first| first.id == comment.id);
        let is_last = !matches!(
            self.rows.get(index + 1),
            Some(PaneRow::Comment { thread: next, .. }) if next == &thread.id
        );
        let is_draft = comment.round.is_none();
        let state = match comment.round {
            Some(round) => format!("round-{round}"),
            None => "draft".to_owned(),
        };
        let (avatar, who_label, badge) = match (comment.author, comment.round) {
            (Author::Agent, Some(round)) => (
                Avatar::agent(),
                "Agent".to_owned(),
                Some(Chip::new(format!("Round {round}"))),
            ),
            (Author::Agent, None) => (Avatar::agent(), "Agent".to_owned(), None),
            (Author::Reviewer, Some(round)) => (
                Avatar::you(),
                "You".to_owned(),
                Some(Chip::new(format!("Round {round}"))),
            ),
            (Author::Reviewer, None) => (
                Avatar::you(),
                "You".to_owned(),
                (thread.comments.len() > 1).then(|| Chip::new("Draft").tone(Tone::Warn)),
            ),
        };

        let edit_tid = thread.id.clone();
        let edit_cid = comment.id.clone();
        let del_cid = comment.id.clone();
        let draft_actions = is_draft.then(|| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    Button::new(
                        SharedString::from(format!("comment-edit-{edit_cid}")),
                        "Edit",
                    )
                    .kind(ButtonKind::Ghost)
                    .small()
                    .on_click(cx.listener(
                        move |pane, _: &ClickEvent, window, cx| {
                            pane.open_edit_draft(edit_tid.clone(), edit_cid.clone(), window, cx);
                        },
                    )),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("comment-delete-{del_cid}")),
                        "Delete",
                    )
                    .kind(ButtonKind::Ghost)
                    .small()
                    .on_click(cx.listener(
                        move |pane, _: &ClickEvent, window, cx| {
                            pane.delete_draft_by_id(del_cid.clone(), window, cx);
                        },
                    )),
                )
        });

        let cid = comment.id.clone();
        let body = div()
            .flex()
            .items_start()
            .gap(SPACE_4)
            .px(SPACE_5)
            .py(SPACE_4)
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(SPACE_1)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_3)
                            .child(
                                div()
                                    .typed(TypeRole::Strong)
                                    .text_color(palette.text)
                                    .child(who_label),
                            )
                            .children(badge)
                            .child(div().flex_1())
                            .children(draft_actions),
                    )
                    .child(
                        div()
                            .debug_selector(|| format!("review-comment-{cid}:{state}"))
                            .w_full()
                            .typed(TypeRole::Body)
                            .text_color(palette.text)
                            .child(comment.text.clone()),
                    ),
            );

        let has_draft = thread.comments.iter().any(|c| c.round.is_none());
        let has_sent = thread.comments.iter().any(|c| c.round.is_some());
        let footer = (is_last && !thread.resolved).then(|| {
            let reply_tid = thread.id.clone();
            let attach_tid = thread.id.clone();
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .px(SPACE_5)
                .py(SPACE_3)
                .border_t_1()
                .border_color(palette.separator)
                .when(!has_draft && has_sent, |row| {
                    row.child(
                        Button::new(
                            SharedString::from(format!("thread-reply-{reply_tid}")),
                            "Reply",
                        )
                        .small()
                        .on_click(cx.listener(
                            move |pane, _: &ClickEvent, window, cx| {
                                pane.open_reply(reply_tid.clone(), window, cx);
                            },
                        )),
                    )
                })
                .child(
                    Button::new(
                        SharedString::from(format!("thread-attach-{attach_tid}")),
                        "Attach only this",
                    )
                    .kind(ButtonKind::Ghost)
                    .small()
                    .on_click(cx.listener(
                        move |pane, _: &ClickEvent, _, cx| {
                            pane.attach_only_thread(attach_tid.clone(), cx);
                        },
                    )),
                )
        });

        let mut card = Card::new()
            .elevated()
            .accent(thread_accent(thread.state, &palette))
            .part(if is_last {
                CardPart::Bottom
            } else {
                CardPart::Middle
            })
            .max_w(px(760.));
        if !is_first {
            card = card.child(Divider::horizontal());
        }
        card = card.child(body).children(footer);

        div()
            .debug_selector(|| format!("review-row-{index}"))
            .w_full()
            .pl(self.thread_indent())
            .pr(SPACE_6)
            .when(is_last, |el| el.pb(SPACE_4))
            .child(card)
            .into_any_element()
    }

    fn draw_diff(
        &self,
        row: Div,
        index: usize,
        path: &Path,
        shown: &Row,
        focused: bool,
        cx: &Context<Self>,
    ) -> Div {
        let palette = palette(cx);
        let Some(diff) = self.diff_of(path) else {
            return row;
        };
        let on = self
            .position
            .as_ref()
            .filter(|_| self.position_row == Some(index))
            .map(|position| position.side);
        let active_bar = cursor_bar_colour(focused, &palette);
        let focus_bar = |here: bool| {
            if here {
                active_bar
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
        let name = on.and_then(|_| self.name_range());
        let h_scroll = self.h_scroll;
        let text = |side: Side, line: u32, kind: DiffKind, name: Option<Range<usize>>| {
            line_text(
                diff,
                path,
                (index, side, line),
                kind,
                name,
                h_scroll,
                &palette,
            )
        };
        let go = |side: Side, layout: TextLayout| {
            cx.listener(move |pane, event: &MouseDownEvent, window, cx| {
                pane.clicked_row(index, side, &layout, event, window, cx);
            })
        };
        let rest = |side: Side, layout: TextLayout| {
            cx.listener(move |pane, event: &MouseMoveEvent, window, cx| {
                pane.moved_over(Spot::Diff { index, side }, &layout, event, window, cx);
            })
        };
        let group_name = SharedString::from(format!("diff-row-{index}"));
        let gutter = |side: Side, active: bool| {
            div()
                .flex_none()
                .w(GUTTER_WIDTH)
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    GutterPlus::new(("diff-plus", index * 2 + usize::from(side == Side::New)))
                        .visible(active)
                        .reveal_on_group_hover(group_name.clone())
                        .tooltip("Add comment (c)")
                        .on_click(cx.listener(move |pane, _: &ClickEvent, window, cx| {
                            pane.open_comment_at(index, side, window, cx);
                        })),
                )
        };

        match (self.layout, shown) {
            (Layout::Split, Row::Skipped { old, .. }) => row
                .justify_center()
                .px(SPACE_6)
                .bg(palette.fold_ground)
                .typed(TypeRole::Meta)
                .text_color(palette.text_dim)
                .child(match old.len() {
                    1 => "⋯ 1 unchanged line".to_owned(),
                    lines => format!("⋯ {lines} unchanged lines"),
                }),
            (Layout::Unified, Row::Skipped { old, .. }) => row
                .pl(px(115.))
                .pr(SPACE_6)
                .bg(palette.fold_ground)
                .typed(TypeRole::Meta)
                .text_color(palette.text_dim)
                .child(match old.len() {
                    1 => "⋯ 1 unchanged line".to_owned(),
                    lines => format!("⋯ {lines} unchanged lines"),
                }),
            (Layout::Unified, _) => {
                // The sign repeats the colour, for a reader who cannot tell the two colours apart.
                let (side, sign, kind) = match shown {
                    Row::Removed { .. } => (Side::Old, "-", DiffKind::Removed),
                    Row::Added { .. } => (Side::New, "+", DiffKind::Added),
                    _ => (Side::New, "", DiffKind::Context),
                };
                let Some(line) = shown.line(side) else {
                    return row;
                };
                let (text, layout) = text(side, line, kind, name);
                let row = row
                    .group(group_name.clone())
                    .border_l_2()
                    .border_color(focus_bar(on.is_some()))
                    .on_mouse_down(MouseButton::Left, go(side, layout.clone()))
                    .on_mouse_move(rest(side, layout))
                    .child(gutter(side, on.is_some()))
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
                    .child(text);
                match ground(kind, &palette) {
                    Some(tint) => row.bg(tint),
                    None => row,
                }
            }
            (Layout::Split, _) => {
                // An unchanged line is on both sides and the reviewer is on it as a whole; a
                // changed row holds two lines.
                let unchanged = matches!(shown, Row::Context { .. });
                let half = |side: Side, kind: DiffKind| {
                    let half = div()
                        .debug_selector(|| format!("review-row-{index}-{}", side_name(side)))
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
                    let name = name
                        .clone()
                        .filter(|_| on == Some(stop) && (!unchanged || side == Side::New));
                    let (text, layout) = text(side, line, line_kind, name);
                    let half = half
                        .border_color(focus_bar(here))
                        .on_mouse_down(MouseButton::Left, go(stop, layout.clone()))
                        .on_mouse_move(rest(stop, layout))
                        .child(gutter(stop, here && (!unchanged || side == Side::New)))
                        .child(number(Some(line)))
                        .child(bar(line_kind, &palette))
                        .child(text);
                    match ground(line_kind, &palette) {
                        Some(tint) => half.bg(tint),
                        None => half,
                    }
                };
                row.group(group_name.clone())
                    .child(half(Side::Old, DiffKind::Removed))
                    .child(Divider::vertical())
                    .child(half(Side::New, DiffKind::Added))
            }
        }
    }
}

pub(crate) type Colour = Hsla;

pub(crate) fn thread_accent(state: ThreadState, palette: &Palette) -> Colour {
    match state {
        ThreadState::Draft | ThreadState::WaitsForReviewer => palette.warn,
        ThreadState::Addressed | ThreadState::Resolved => palette.ok,
        ThreadState::Sent => palette.link,
    }
}

pub(crate) fn cursor_bar_colour(focused: bool, palette: &Palette) -> Colour {
    if focused { palette.focus } else { palette.edge }
}

pub(crate) fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "old",
        Side::New => "new",
    }
}

/// Merges syntax-colour spans with optional word-diff background spans into sorted, disjoint
/// highlight ranges for GPUI's `StyledText`.
pub(crate) fn diff_highlights(
    path: &Path,
    drawn: &str,
    word_marks: &[Range<usize>],
    word_bg: Option<Hsla>,
    palette: &Palette,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let end = drawn.len();
    let syntax_spans = syntax::highlight_line(path, drawn);
    let words: Vec<Range<usize>> = if word_bg.is_some() {
        word_marks
            .iter()
            .filter(|mark| mark.start < end && mark.start < mark.end)
            .map(|mark| mark.start..mark.end.min(end))
            .collect()
    } else {
        Vec::new()
    };
    if syntax_spans.is_empty() && words.is_empty() {
        return Vec::new();
    }
    if words.is_empty() {
        return syntax_spans
            .into_iter()
            .map(|(range, kind)| {
                (
                    range,
                    HighlightStyle {
                        color: Some(syntax::colour(kind, palette)),
                        ..HighlightStyle::default()
                    },
                )
            })
            .collect();
    }
    let mut cuts = Vec::with_capacity((syntax_spans.len() + words.len()) * 2);
    for (range, _) in &syntax_spans {
        cuts.push(range.start);
        cuts.push(range.end);
    }
    for range in &words {
        cuts.push(range.start);
        cuts.push(range.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a >= b {
            continue;
        }
        let in_word = words.iter().any(|mark| mark.start <= a && b <= mark.end);
        let syn_kind = syntax_spans
            .iter()
            .find(|(range, _)| range.start <= a && b <= range.end)
            .map(|(_, kind)| *kind);
        let style = if in_word {
            // Text on a word chip is always `palette.text`, never dim, for contrast.
            HighlightStyle {
                color: Some(palette.text),
                background_color: word_bg,
                ..HighlightStyle::default()
            }
        } else if let Some(kind) = syn_kind {
            HighlightStyle {
                color: Some(syntax::colour(kind, palette)),
                ..HighlightStyle::default()
            }
        } else {
            continue;
        };
        out.push((a..b, style));
    }
    out
}

/// The selector carries the text as handed to the text system: a test cannot read what is drawn.
fn line_text(
    diff: &ShownDiff,
    path: &Path,
    (index, side, line): (usize, Side, u32),
    kind: DiffKind,
    name: Option<Range<usize>>,
    h_scroll: Pixels,
    palette: &Palette,
) -> (Div, TextLayout) {
    let text = diff.text(side, line);
    // Marks count the bytes of the line as in the file. Where non-text bytes were replaced, the
    // shown text differs and nothing is marked.
    let marks = match text {
        Cow::Borrowed(_) => diff.marks(side, line),
        Cow::Owned(_) => Vec::new(),
    };
    let end = text.floor_char_boundary(DRAWN_BYTES);
    let drawn = text[..end].to_owned();
    let highlights = diff_highlights(path, &drawn, &marks, word(kind, palette), palette);
    let name = name.filter(|name| name.end <= end);
    code_text(
        format!("review-text-{index}-{}:{drawn}", side_name(side)),
        &drawn,
        highlights,
        name,
        h_scroll,
        palette.focus,
        palette.text,
    )
}

/// The text element is as wide as the text, which a test reads. The name the reviewer is on is
/// marked by an invisible copy of the line laid over it, whose piece for the name is underlined.
pub(crate) fn code_text(
    selector: String,
    text: &str,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    name: Option<Range<usize>>,
    h_scroll: Pixels,
    name_mark: Hsla,
    text_colour: Hsla,
) -> (Div, TextLayout) {
    let styled = StyledText::new(text.to_owned()).with_highlights(highlights);
    let layout = styled.layout().clone();
    let over = name.map(|name| {
        let before = &text[..name.start];
        div()
            .absolute()
            .top_0()
            .left_0()
            .h_full()
            .flex()
            .whitespace_nowrap()
            .text_color(transparent_black())
            .children((!before.is_empty()).then(|| div().child(before.to_owned())))
            .child(
                div()
                    .debug_selector(|| "review-name".to_owned())
                    .border_b_2()
                    .border_color(name_mark)
                    .child(text[name].to_owned()),
            )
    });
    let element = div()
        .debug_selector(move || selector.clone())
        .relative()
        .flex_none()
        .whitespace_nowrap()
        .typed(TypeRole::Code)
        .text_color(text_colour)
        .when(h_scroll > px(0.), |el| el.ml(-h_scroll))
        .child(styled)
        .children(over);
    (
        div()
            .flex_1()
            .min_w_0()
            .px(SPACE_2)
            .flex()
            .overflow_hidden()
            .child(element),
        layout,
    )
}

pub(crate) fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

fn base_words(base: &Base) -> String {
    // A base that is not a branch is the full id of a commit.
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

pub(crate) fn comparison_sentence(comparison: &Comparison) -> String {
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

pub(crate) fn status_words(status: &FileStatus) -> String {
    match status {
        FileStatus::Added => "new".to_owned(),
        FileStatus::Modified => "changed".to_owned(),
        FileStatus::Deleted => "deleted".to_owned(),
        FileStatus::Renamed { from } => format!("was {}", from.display()),
    }
}

pub(crate) fn unshown_sentence(why: Unshown) -> &'static str {
    match why {
        Unshown::Loading => "Reading the file",
        Unshown::Binary => "Not text: its lines are not shown",
        Unshown::TooLarge => "Too large to show",
        Unshown::Refused => "The Keeper could not read this file",
    }
}

#[cfg(test)]
mod tests {
    use yaar_link::ThreadState;
    use yaar_ui::themes::{DARK, Themes};

    use super::{cursor_bar_colour, thread_accent};

    #[test]
    fn unfocused_diff_cursor_bar_dims_to_edge() {
        let palette = Themes::built_in().named(DARK).expect("dark theme").palette;
        assert_eq!(cursor_bar_colour(true, &palette), palette.focus);
        assert_eq!(cursor_bar_colour(false, &palette), palette.edge);
        assert_ne!(palette.focus, palette.edge);
    }

    #[test]
    fn thread_card_accent_matches_thread_state() {
        let palette = Themes::built_in().named(DARK).expect("dark theme").palette;
        assert_eq!(thread_accent(ThreadState::Draft, &palette), palette.warn);
        assert_eq!(
            thread_accent(ThreadState::WaitsForReviewer, &palette),
            palette.warn
        );
        assert_eq!(thread_accent(ThreadState::Addressed, &palette), palette.ok);
        assert_eq!(thread_accent(ThreadState::Resolved, &palette), palette.ok);
        assert_eq!(thread_accent(ThreadState::Sent, &palette), palette.link);
    }
}
