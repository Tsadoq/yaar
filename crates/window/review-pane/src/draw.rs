use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::{Hunk, Row, Side};
use yaar_frame::SetBase;
use yaar_link::{Author, Base, BaseSource, Comparison, FileStatus, NoComparison};

use crate::navigate::Spot;
use crate::{ChangeShown, KEY_CONTEXT, Layout, PaneRow, ReviewPane, ShownDiff, Unshown, View};
use yaar_link::{CommentId, ThreadId};

pub(crate) const ROW_HEIGHT: Pixels = px(20.);

pub(crate) const NUMBER_WIDTH: Pixels = px(56.);

const SIGN_WIDTH: Pixels = px(20.);

/// A line is never wrapped and the overflow is cut off, so the rest of a megabyte line would be
/// shaped for nothing in every frame.
const DRAWN_BYTES: usize = 1024;

impl ShownDiff {
    pub(crate) fn text(&self, side: Side, line: u32) -> Cow<'_, str> {
        String::from_utf8_lossy(self.diff.line(side, line))
    }

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_cursor();
        // Outside the element that holds the pane's key context: the keys of the tree are not the
        // keys of the rows.
        let tree = self.tree_view(window, cx);
        let sessions = self.session_bar(cx);
        let offer = self.offer_bar(cx);
        let start = self.render_offer_to_start(cx);
        let rounds = self.render_rounds(cx);
        let bar = self.render_bar(window, cx);
        let servers = self.render_servers(cx);
        let hover = self.render_hover(cx);
        let references = self.render_references(cx);
        let file_view = self.render_file_view(window, cx);
        let flow = (self.view == View::Flow).then(|| self.render_flow(window, cx));
        let theme = cx.theme();
        let pane = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.mono_font_family.clone())
            .text_size(theme.mono_font_size);
        // A mouse button or the wheel here, or a key no action takes, pauses follow. The bar is
        // left out: its controls choose what Flow shows.
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .capture_any_mouse_down(cx.listener(|pane, _: &MouseDownEvent, _, cx| {
                pane.touched(cx);
            }))
            .capture_key_down(cx.listener(|pane, _: &KeyDownEvent, _, cx| pane.touched(cx)))
            .on_scroll(cx.listener(|pane, _: &ScrollEvent, _, cx| {
                pane.scrolled();
                pane.touched(cx);
            }));
        // The notice and the comment box go below the rows, outside the element that sets the key
        // context, so that letters typed in the box are text, not keys of the pane.
        let notice = self.notice.as_ref().map(|notice| {
            div()
                .debug_selector(|| "review-notice".to_owned())
                .flex_none()
                .h(ROW_HEIGHT)
                .px_2()
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .border_t_1()
                .border_color(theme.border)
                .text_color(theme.danger)
                .child(notice.clone())
        });
        let comment_box = self.open_box.as_ref().map(|open| {
            div()
                .debug_selector(|| "review-comment-box".to_owned())
                .flex_none()
                .p_2()
                .border_t_1()
                .border_color(theme.border)
                // The field lets Escape pass once it has nothing of its own to close.
                .on_action(cx.listener(Self::escape))
                .child(Textarea::new(&open.input))
        });
        let commit_field = self.commit_field.as_ref().map(|field| {
            div()
                .debug_selector(|| "review-commit-field".to_owned())
                .flex_none()
                .p_2()
                .border_t_1()
                .border_color(theme.border)
                .on_action(cx.listener(Self::escape_commit))
                .child(Input::new(&field.input))
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
            let idle = div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child(self.idle_line(cx));
            return pane.children(bar).children(servers).child(
                body.children(sessions)
                    .children(start)
                    .children(rounds)
                    .children(offer)
                    .children(tree)
                    .child(self.with_keys(idle, cx))
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
        // The rows stay while the Keeper refuses to read the changes again, and may be out of date:
        // the reviewer is told.
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
        let list = uniform_list(
            "review-rows",
            self.rows.len(),
            cx.processor(|pane, rows: Range<usize>, _, cx| {
                rows.map(|index| pane.draw_row(index, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .with_decoration(InView(self.in_view.clone()))
        .size_full();
        pane.children(bar).children(servers).child(
            body.children(sessions)
                .children(start)
                .children(rounds)
                .children(offer)
                .children(tree)
                .children(refused)
                .child(self.with_keys(div().flex_1().min_h_0().child(list), cx))
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
        action: &crate::ToggleFlowForm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::Flow {
            self.toggle_flow_form(action, window, cx);
        }
    }

    fn resume_follow_in_flow(
        &mut self,
        action: &crate::ResumeFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::Flow {
            self.resume_follow(action, window, cx);
        }
    }

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
                // The sentence gives way where the row is too narrow, so the offer is never pushed
                // out.
                let row = row.px_2().gap_3().bg(theme.muted).child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(comparison_sentence(comparison)),
                );
                let reviewed = self
                    .review()
                    .and_then(|review| review.reviewed.as_deref())
                    .map(|state| format!("Reviewed at {}", short(state)));
                let row = row.children(reviewed.map(|words| {
                    div()
                        .debug_selector(|| "review-reviewed".to_owned())
                        .flex_none()
                        .text_color(theme.success)
                        .child(words)
                }));
                if !offers_base {
                    return row;
                }
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
            PaneRow::File { path, status } => {
                let viewed = self
                    .reviews()
                    .is_some_and(|reviews| reviews.viewed.contains(path));
                let no_agent =
                    self.tree.shown == ChangeShown::Uncommitted && self.made_by_no_agent(path);
                let shared = self.shared_mark(index, path, cx);
                row.px_2()
                    .gap_3()
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .child(path.display().to_string())
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .child(status_words(status)),
                    )
                    .children(no_agent.then(|| {
                        div()
                            .debug_selector(|| format!("review-no-agent-{index}"))
                            .flex_none()
                            .text_color(theme.warning)
                            .child("no agent")
                    }))
                    .children(shared)
                    .children(viewed.then(|| {
                        div()
                            .debug_selector(|| format!("review-viewed-{index}"))
                            .flex_none()
                            .text_color(theme.success)
                            .child("viewed")
                    }))
            }
            PaneRow::Unshown { why, .. } => row
                .px_2()
                .text_color(theme.muted_foreground)
                .child(unshown_sentence(*why)),
            PaneRow::Diff { path, row: shown } => self.draw_diff(row, index, path, shown, cx),
            PaneRow::Thread { thread } => self.draw_thread(row, thread, cx),
            PaneRow::Comment { thread, comment } => self.draw_comment(row, thread, comment),
            PaneRow::ThreadsElsewhere => row
                .px_2()
                .bg(theme.secondary)
                .text_color(theme.secondary_foreground)
                .child(
                    div()
                        .debug_selector(|| "review-threads-elsewhere".to_owned())
                        .child("Threads elsewhere: their lines are in no row shown"),
                ),
            PaneRow::Read { path } => row
                .px_2()
                .text_color(theme.muted_foreground)
                .child(path.display().to_string()),
        }
    }

    pub(crate) fn draw_thread(&self, row: Div, thread: &ThreadId, cx: &Context<Self>) -> Div {
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
        let state = self.thread_state_mark(&thread.id, cx);
        let moved = self.thread_moved_marks(&thread.id, cx);
        row.pl_6()
            .pr_2()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .debug_selector(|| format!("review-thread-{id}"))
                    .child(format!(
                        "Thread on {}:{place} ({side}){}",
                        anchor.path.display(),
                        if thread.resolved { ", resolved" } else { "" }
                    )),
            )
            .children(state)
            .children(moved)
    }

    pub(crate) fn draw_comment(&self, row: Div, thread: &ThreadId, comment: &CommentId) -> Div {
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
                .debug_selector(|| format!("review-comment-{id}:{state}"))
                .child(format!("{who}: {first}{more}")),
        )
    }

    fn draw_diff(
        &self,
        row: Div,
        index: usize,
        path: &Path,
        shown: &Row,
        cx: &Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let Some(diff) = self.diff_of(path) else {
            return row;
        };
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
                .child(line.map(|line| (line + 1).to_string()).unwrap_or_default())
        };
        let name = on.and_then(|_| self.name_range());
        let text = |side: Side, line: u32, name: Option<Range<usize>>| {
            let mark = match side {
                Side::Old => theme.danger.opacity(0.4),
                Side::New => theme.success.opacity(0.4),
            };
            line_text(diff, index, side, line, mark, name, theme.primary)
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
                // The sign repeats the colour, for a reader who cannot tell the two colours apart.
                let (side, sign, tint) = match shown {
                    Row::Removed { .. } => (Side::Old, "-", Some(removed)),
                    Row::Added { .. } => (Side::New, "+", Some(added)),
                    _ => (Side::New, "", None),
                };
                let Some(line) = shown.line(side) else {
                    return row;
                };
                let (text, layout) = text(side, line, name);
                let row = row
                    .border_l_2()
                    .border_color(bar(on.is_some()))
                    .on_mouse_down(MouseButton::Left, go(side, layout.clone()))
                    .on_mouse_move(rest(side, layout))
                    .child(number(shown.line(Side::Old)))
                    .child(number(shown.line(Side::New)))
                    .child(div().flex_none().w(SIGN_WIDTH).px_1().child(sign))
                    .child(text);
                match tint {
                    Some(tint) => row.bg(tint),
                    None => row,
                }
            }
            (Layout::Split, _) => {
                // An unchanged line is on both sides and the reviewer is on it as a whole; a
                // changed row holds two lines.
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
                        return half
                            .border_color(transparent_black())
                            .bg(theme.muted.opacity(0.5));
                    };
                    let (stop, here) = if unchanged {
                        (Side::New, on.is_some())
                    } else {
                        (side, on == Some(side))
                    };
                    let name = name
                        .clone()
                        .filter(|_| on == Some(stop) && (!unchanged || side == Side::New));
                    let (text, layout) = text(side, line, name);
                    let half = half
                        .border_color(bar(here))
                        .on_mouse_down(MouseButton::Left, go(stop, layout.clone()))
                        .on_mouse_move(rest(stop, layout))
                        .child(number(Some(line)))
                        .child(text);
                    if unchanged { half } else { half.bg(tint) }
                };
                row.child(half(Side::Old, removed))
                    .child(half(Side::New, added))
            }
        }
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Old => "old",
        Side::New => "new",
    }
}

/// The selector carries the text as handed to the text system: a test cannot read what is drawn.
fn line_text(
    diff: &ShownDiff,
    index: usize,
    side: Side,
    line: u32,
    mark: Hsla,
    name: Option<Range<usize>>,
    name_mark: Hsla,
) -> (Div, TextLayout) {
    let text = diff.text(side, line);
    // Marks count the bytes of the line as in the file. Where non-text bytes were replaced, the
    // shown text differs and nothing is marked.
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
    let name = name.filter(|name| name.end <= end);
    code_text(
        format!("review-text-{index}-{}:{drawn}", side_name(side)),
        &drawn,
        highlights,
        name,
        name_mark,
    )
}

/// The text element is as wide as the text, which a test reads. The name the reviewer is on is
/// marked by an invisible copy of the line laid over it, whose piece for the name is underlined.
pub(crate) fn code_text(
    selector: String,
    text: &str,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    name: Option<Range<usize>>,
    name_mark: Hsla,
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
        .child(styled)
        .children(over);
    (
        div()
            .flex_1()
            .min_w_0()
            .px_1()
            .flex()
            .overflow_hidden()
            .child(element),
        layout,
    )
}

fn short(commit: &str) -> &str {
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

fn status_words(status: &FileStatus) -> String {
    match status {
        FileStatus::Added => "new".to_owned(),
        FileStatus::Modified => "changed".to_owned(),
        FileStatus::Deleted => "deleted".to_owned(),
        FileStatus::Renamed { from } => format!("was {}", from.display()),
    }
}

fn unshown_sentence(why: Unshown) -> &'static str {
    match why {
        Unshown::Loading => "Reading the file",
        Unshown::Binary => "Not text: its lines are not shown",
        Unshown::TooLarge => "Too large to show",
        Unshown::Refused => "The Keeper could not read this file",
    }
}
