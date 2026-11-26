//! Read-only view of a file reached by go to definition or a reference. A comment on a line anchors
//! by file kind:
//!
//! | File | `Anchor::tree` | `Anchor::path` | `Anchor::side` |
//! | --- | --- | --- | --- |
//! | worktree | none | relative | new |
//! | export (`Version`) | the version | relative | old |
//! | outside or git-ignored | none | absolute | new |

use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;
use yaar_diff::Side;
use yaar_link::{Anchor, CommentId, Lines, Side as LinkSide, Thread, ThreadId};
use yaar_navigator::{FileKind, Location, OpenedFile};
use yaar_ui::metrics::{SPACE_3, SPACE_4, SPACE_6, tabular};
use yaar_ui::pieces::{Button, ButtonKind};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::draw::{NUMBER_WIDTH, ROW_HEIGHT, code_text, diff_highlights};
use crate::navigate::{Left, Spot};
use crate::saves::new_id;
use crate::threads::{Place, place_of};
use crate::{CommentBox, Position, ReviewPane};

pub const BACK: &str = "Back to the diff";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileShown {
    pub file: OpenedFile,
    /// From 1.
    pub line: u32,
}

enum FileItem {
    Line(u32),
    Thread(ThreadId),
    Comment(ThreadId, CommentId),
}

fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let kept = line.strip_suffix('\n').unwrap_or(line);
        let kept = kept.strip_suffix('\r').unwrap_or(kept);
        ranges.push(start..start + kept.len());
        start = end;
    }
    if ranges.is_empty() {
        ranges.push(0..0);
    }
    ranges
}

impl ReviewPane {
    pub fn file_view(&self) -> Option<&FileShown> {
        self.navigate.file.as_ref()
    }

    pub(crate) fn file_line(&self, line: u32) -> Option<&str> {
        let file = self.navigate.file.as_ref()?;
        let range = self
            .navigate
            .file_lines
            .get(line.checked_sub(1)? as usize)?;
        file.file.text.get(range.clone())
    }

    pub(crate) fn open_location(
        &mut self,
        location: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = location.line;
        let asking = self
            .models
            .navigation
            .update(cx, |navigation, cx| navigation.open(location, cx));
        self.navigate.opening = Some(cx.spawn_in(window, async move |pane, cx| {
            let answer = asking.await;
            pane.update_in(cx, |pane, window, cx| match answer {
                Ok(Ok(file)) => pane.show_file(file, line, window, cx),
                Ok(Err(why)) => pane.say(why, cx),
                Err(error) => pane.say(error.to_string(), cx),
            })
            .ok();
        }));
    }

    /// The place left in the diff is kept only when the diff is what is shown.
    fn show_file(
        &mut self,
        file: OpenedFile,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.follow_cursor();
        if self.navigate.file.is_none() {
            self.navigate.left = Some(Left {
                view: self.view,
                name: self.navigate.name.clone(),
            });
        }
        self.navigate.file_lines = line_ranges(&file.text);
        let line = line.clamp(1, self.navigate.file_lines.len() as u32);
        self.navigate.file = Some(FileShown { file, line });
        self.navigate.name = None;
        self.close_list(window, cx);
        self.focus.focus(window, cx);
        self.navigate
            .file_scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(0.)));
        self.scroll_file_to(line, ScrollStrategy::Center);
        self.follow_cursor();
        cx.notify();
    }

    fn scroll_file_to(&self, line: u32, strategy: ScrollStrategy) {
        let items = self.file_items();
        let item = items
            .iter()
            .position(|item| matches!(item, FileItem::Line(at) if *at == line))
            .unwrap_or(0);
        self.navigate.file_scroll.scroll_to_item(item, strategy);
    }

    pub(crate) fn scroll_file_to_comment(&self, thread: &ThreadId, comment: &CommentId) {
        if self.navigate.file.is_none() {
            return;
        }
        let items = self.file_items();
        if let Some(item) = items
            .iter()
            .position(|item| matches!(item, FileItem::Comment(t, c) if t == thread && c == comment))
        {
            self.navigate
                .file_scroll
                .scroll_to_item(item, ScrollStrategy::Nearest);
        }
    }

    pub(crate) fn go_back(
        &mut self,
        _: &crate::GoBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_session_picker(cx) {
            return;
        }
        self.acted(cx);
        self.follow_cursor();
        if self.drop_hover() {
            cx.notify();
            return;
        }
        self.leave_file_view(window, cx);
    }

    /// The scroll of the rows is not restored: their scroll handle keeps its offset.
    fn leave_file_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigate.file.take().is_none() {
            return;
        }
        self.navigate.file_lines.clear();
        if let Some(left) = self.navigate.left.take() {
            self.view = left.view;
            self.navigate.name = left.name;
        }
        self.close_list(window, cx);
        self.focus.focus(window, cx);
        self.follow_cursor();
        cx.notify();
    }

    fn file_items(&self) -> Vec<FileItem> {
        let Some(shown) = &self.navigate.file else {
            return Vec::new();
        };
        let threads = self.threads_in_file(shown);
        let mut items = Vec::with_capacity(self.navigate.file_lines.len() + threads.len());
        for line in 1..=self.navigate.file_lines.len() as u32 {
            items.push(FileItem::Line(line));
            for thread in threads.get(&line).into_iter().flatten() {
                items.push(FileItem::Thread(thread.id.clone()));
                items.extend(
                    thread
                        .comments
                        .iter()
                        .map(|comment| FileItem::Comment(thread.id.clone(), comment.id.clone())),
                );
            }
        }
        items
    }

    fn threads_in_file(&self, shown: &FileShown) -> BTreeMap<u32, Vec<&Thread>> {
        let mut by_line: BTreeMap<u32, Vec<&Thread>> = BTreeMap::new();
        let Some(review) = self.review() else {
            return by_line;
        };
        let (relative, side, version) = match &shown.file.kind {
            FileKind::Worktree { relative, .. } => (relative, LinkSide::New, None),
            FileKind::Version {
                relative, version, ..
            } => (relative, LinkSide::Old, Some(version.as_str())),
            FileKind::Outside => (&shown.file.path, LinkSide::New, None),
        };
        for thread in &review.threads {
            // A thread of a version is on the lines of that version only; the others sit where the
            // Keeper carried them to in the files on disk.
            if version.is_some_and(|version| thread.anchor.tree.as_deref() != Some(version)) {
                continue;
            }
            let (first, last, path, at) = match place_of(thread, version) {
                Some(Place::Anchor(thread)) => (
                    thread.anchor.lines.first,
                    thread.anchor.lines.last,
                    &thread.anchor.path,
                    thread.anchor.side,
                ),
                Some(Place::Now(now)) => (now.lines.first, now.lines.last, &now.path, now.side),
                None => continue,
            };
            if path != relative || at != side || first == 0 || last < first {
                continue;
            }
            let held: Vec<&str> = (first..=last)
                .map_while(|line| self.file_line(line))
                .collect();
            if held.len() == (last - first + 1) as usize && held.join("\n") == thread.anchor.text {
                by_line.entry(last).or_default().push(thread);
            }
        }
        by_line
    }

    pub(crate) fn comment_in_file_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(shown) = self.navigate.file.clone() else {
            return false;
        };
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            self.say("The reviews are not here yet", cx);
            return true;
        };
        let Some(text) = self.file_line(shown.line).map(str::to_owned) else {
            self.say("The text of the line is not here", cx);
            return true;
        };
        let (tree, path, side) = match &shown.file.kind {
            FileKind::Worktree { relative, .. } => (None, relative.clone(), Side::New),
            FileKind::Version {
                version, relative, ..
            } => (Some(version.clone()), relative.clone(), Side::Old),
            FileKind::Outside => (None, shown.file.path.clone(), Side::New),
        };
        let (Ok(thread), Ok(comment)) = (new_id(), new_id()) else {
            self.say("An id for the comment could not be drawn", cx);
            return true;
        };
        let anchor = Anchor {
            tree,
            path: path.clone(),
            side: crate::threads::link_side(side),
            lines: Lines {
                first: shown.line,
                last: shown.line,
            },
            text,
        };
        self.open_box(
            CommentBox {
                review,
                thread: ThreadId::new(thread),
                comment: CommentId::new(comment),
                anchor: Some(anchor),
                at: Position {
                    path,
                    side,
                    line: shown.line - 1,
                },
                text: String::new(),
            },
            window,
            cx,
        );
        self.scroll_file_to(shown.line, ScrollStrategy::Center);
        true
    }

    fn clicked_line(
        &mut self,
        line: u32,
        layout: &TextLayout,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(shown) = &mut self.navigate.file {
            shown.line = line;
        }
        self.clicked(Spot::File(line), layout, event, window, cx);
    }

    pub(crate) fn render_file_view(&self, cx: &mut Context<Self>) -> Option<Div> {
        let shown = self.navigate.file.as_ref()?;
        let palette = palette(cx);
        let path = match &shown.file.kind {
            FileKind::Worktree { relative, .. } | FileKind::Version { relative, .. } => {
                relative.display().to_string()
            }
            FileKind::Outside => shown.file.path.display().to_string(),
        };
        let back = Button::new("review-file-back", BACK)
            .kind(ButtonKind::Secondary)
            .small()
            .selector("review-file-back")
            .label_selector(format!("review-file-back:{BACK}"))
            .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                pane.acted(cx);
                pane.leave_file_view(window, cx);
            }));
        let header = div()
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_6)
            .py(SPACE_3)
            .overflow_hidden()
            .whitespace_nowrap()
            .bg(palette.pane)
            .border_b_1()
            .border_color(palette.separator)
            .child(
                div()
                    .debug_selector({
                        let path = path.clone();
                        move || format!("review-file-path:{path}")
                    })
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typed(TypeRole::Path)
                    .text_color(palette.text)
                    .child(path),
            )
            .child(back);
        let items = Rc::new(self.file_items());
        let list = uniform_list(
            "review-file-lines",
            items.len(),
            cx.processor(move |pane, range: Range<usize>, _, cx| {
                range
                    .map(|index| pane.draw_file_item(&items[index], cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.navigate.file_scroll)
        .size_full();
        Some(
            div()
                .debug_selector(|| "review-file-view".to_owned())
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(div().flex_1().min_h_0().child(list)),
        )
    }

    fn draw_file_item(&self, item: &FileItem, cx: &Context<Self>) -> AnyElement {
        let palette = palette(cx);
        match item {
            FileItem::Thread(thread) => self.draw_file_thread(thread, cx),
            FileItem::Comment(thread, comment) => self.draw_file_comment(thread, comment, cx),
            FileItem::Line(line) => {
                let line = *line;
                let here = self
                    .navigate
                    .file
                    .as_ref()
                    .is_some_and(|shown| shown.line == line);
                let text = self.file_line(line).unwrap_or_default();
                let end = text.floor_char_boundary(1024);
                let name = if here {
                    self.name_range().filter(|name| name.end <= end)
                } else {
                    None
                };
                let highlights = self
                    .navigate
                    .file
                    .as_ref()
                    .map(|shown| {
                        diff_highlights(&shown.file.path, &text[..end], &[], None, &palette)
                    })
                    .unwrap_or_default();
                let (text_element, layout) = code_text(
                    format!("review-file-text-{line}:{}", &text[..end]),
                    &text[..end],
                    highlights,
                    name,
                    px(0.),
                    palette.focus,
                    palette.text,
                );
                let marker = if here {
                    div()
                        .debug_selector(|| "review-file-at".to_owned())
                        .bg(palette.focus)
                } else {
                    div()
                };
                div()
                    .h(ROW_HEIGHT)
                    .w_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .debug_selector(move || format!("review-file-line-{line}"))
                    .on_mouse_down(MouseButton::Left, {
                        let layout = layout.clone();
                        cx.listener(move |pane, event: &MouseDownEvent, window, cx| {
                            pane.clicked_line(line, &layout, event, window, cx);
                        })
                    })
                    .on_mouse_move({
                        let layout = layout.clone();
                        cx.listener(move |pane, event: &MouseMoveEvent, window, cx| {
                            pane.moved_over(Spot::File(line), &layout, event, window, cx);
                        })
                    })
                    .child(marker.flex_none().w(px(2.)).h_full())
                    .child(
                        div()
                            .flex_none()
                            .w(NUMBER_WIDTH)
                            .pr(SPACE_4)
                            .flex()
                            .justify_end()
                            .typed(TypeRole::CodeSmall)
                            .font_features(tabular())
                            .text_color(palette.text_dim)
                            .child(line.to_string()),
                    )
                    .child(text_element)
                    .into_any_element()
            }
        }
    }

    fn draw_file_thread(&self, thread: &ThreadId, cx: &Context<Self>) -> AnyElement {
        let palette = palette(cx);
        let row = div()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .pl(px(56.))
            .pr(SPACE_6)
            .overflow_hidden()
            .whitespace_nowrap()
            .bg(palette.window);
        let Some(held) = self
            .review()
            .and_then(|review| review.threads.iter().find(|t| t.id == *thread))
        else {
            return row.into_any_element();
        };
        let anchor = &held.anchor;
        let side = match anchor.side {
            LinkSide::Old => "old",
            LinkSide::New => "new",
        };
        let place = if anchor.lines.first == anchor.lines.last {
            anchor.lines.last.to_string()
        } else {
            format!("{}-{}", anchor.lines.first, anchor.lines.last)
        };
        let id = held.id.clone();
        row.child(
            div()
                .debug_selector(|| format!("review-thread-{id}"))
                .typed(TypeRole::Meta)
                .text_color(palette.link)
                .child(format!(
                    "Thread on {}:{place} ({side}){}",
                    anchor.path.display(),
                    if held.resolved { ", resolved" } else { "" }
                )),
        )
        .children(self.thread_state_mark(&held.id, cx))
        .into_any_element()
    }

    fn draw_file_comment(
        &self,
        thread: &ThreadId,
        comment: &CommentId,
        cx: &Context<Self>,
    ) -> AnyElement {
        let palette = palette(cx);
        let row = div()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .pl(px(68.))
            .pr(SPACE_6)
            .overflow_hidden()
            .whitespace_nowrap()
            .bg(palette.window);
        let Some(comment) = self
            .review()
            .and_then(|review| review.threads.iter().find(|t| t.id == *thread))
            .and_then(|thread| thread.comments.iter().find(|c| c.id == *comment))
        else {
            return row.into_any_element();
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
        let first = comment.text.lines().next().unwrap_or_default();
        let id = comment.id.clone();
        row.child(
            div()
                .debug_selector(|| format!("review-comment-{id}:{state}"))
                .typed(TypeRole::Body)
                .text_color(palette.text)
                .child(format!("{who}: {first}")),
        )
        .into_any_element()
    }

    pub(crate) fn file_view_step(&mut self, by: i64, cx: &mut Context<Self>) -> bool {
        let count = self.navigate.file_lines.len() as i64;
        let Some(shown) = &mut self.navigate.file else {
            return false;
        };
        shown.line = (i64::from(shown.line) + by).clamp(1, count.max(1)) as u32;
        let line = shown.line;
        self.follow_cursor();
        self.scroll_file_to(line, ScrollStrategy::Nearest);
        cx.notify();
        true
    }
}
