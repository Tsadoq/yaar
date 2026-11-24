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

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::Side;
use yaar_link::{Anchor, CommentId, Lines, Side as LinkSide, Thread, ThreadId};
use yaar_navigator::{FileKind, Location, OpenedFile};

use crate::draw::{NUMBER_WIDTH, ROW_HEIGHT, code_text};
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

    pub(crate) fn go_back(
        &mut self,
        _: &crate::GoBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(crate) fn render_file_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let _ = window;
        let shown = self.navigate.file.as_ref()?;
        let theme = cx.theme();
        let path = match &shown.file.kind {
            FileKind::Worktree { relative, .. } | FileKind::Version { relative, .. } => {
                relative.display().to_string()
            }
            FileKind::Outside => shown.file.path.display().to_string(),
        };
        let back = div()
            .debug_selector(|| "review-file-back".to_owned())
            .flex_none()
            .px_2()
            .rounded_sm()
            .cursor_pointer()
            .bg(theme.primary)
            .text_color(theme.primary_foreground)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                    pane.acted(cx);
                    pane.leave_file_view(window, cx);
                }),
            )
            .child(
                div()
                    .debug_selector(|| format!("review-file-back:{BACK}"))
                    .child(BACK),
            );
        let header = div()
            .flex_none()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .bg(theme.secondary)
            .text_color(theme.secondary_foreground)
            .child(
                div()
                    .debug_selector({
                        let path = path.clone();
                        move || format!("review-file-path:{path}")
                    })
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

    fn draw_file_item(&self, item: &FileItem, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let row = div()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap();
        match item {
            FileItem::Thread(thread) => self.draw_thread(row, thread, cx),
            FileItem::Comment(thread, comment) => self.draw_comment(row, thread, comment),
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
                let (text_element, layout) = code_text(
                    format!("review-file-text-{line}:{}", &text[..end]),
                    &text[..end],
                    Vec::new(),
                    name,
                    theme.primary,
                );
                let marker = if here {
                    div()
                        .debug_selector(|| "review-file-at".to_owned())
                        .bg(theme.primary)
                } else {
                    div()
                };
                row.debug_selector(move || format!("review-file-line-{line}"))
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
                            .px_1()
                            .flex()
                            .justify_end()
                            .text_color(theme.muted_foreground)
                            .child(line.to_string()),
                    )
                    .child(text_element)
            }
        }
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
