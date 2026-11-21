//! The read-only file view: a file reached through go to definition or a reference, shown
//! whole in the place of the rows, with a control that brings the reviewer back to the
//! place in the diff they left (FR-052, FR-053), and comments on any of its lines (FR-075).
//!
//! # Opening and going back
//!
//! A location is opened with `Navigation::open`, which reads its file. The pane then shows
//! the file view ([`FileShown`]) in the place of the rows and of Flow, under the selector
//! `review-file-view`, the reviewer on the location's line: each line under
//! `review-file-line-<n>`, `n` from 1, its text under `review-file-text-<n>:<text>`, the
//! reviewer's line marked `review-file-at`; its path above the lines under
//! `review-file-path:<path>`, relative to the worktree where the file lies in it; and the
//! control [`BACK`] under `review-file-back`, with the text selector
//! `review-file-back:<the label>`. The file view offers no way to edit (FR-053).
//!
//! The place left is kept when the first file view opens from the diff: the view (Changes
//! or Flow), the reviewer's position, the name and the scroll. [`crate::GoBack`]
//! (`escape`) and a click on the control close the file view, put that place back as it
//! was, and drop it. A definition or a reference followed from inside a file view opens
//! the next file view and keeps the place left in the diff: going back always returns to
//! the diff.
//!
//! In the file view `j`, `k` and the arrows move the reviewer's line within the file, the
//! name keys and the three asks work on its lines as on the diff's, the place asked being
//! the file's (`Tree::Worktree` and the relative path for [`yaar_navigator::FileKind::Worktree`],
//! `Tree::Version` for [`yaar_navigator::FileKind::Version`]; a file outside is asked
//! about with `Tree::Worktree` and its absolute path, which the Navigator answers by the
//! server of its language, or by text match).
//!
//! # Comments on its lines
//!
//! [`crate::Comment`] (`c`) on a line of the file view opens the comment box for a new
//! thread on that line, whether or not the line is in the diff (FR-075), with the anchor:
//!
//! | The file | `Anchor::tree` | `Anchor::path` | `Anchor::side` |
//! | --- | --- | --- | --- |
//! | in the worktree (`FileKind::Worktree`) | none: the files on disk | the relative path | new |
//! | in an export (`FileKind::Version`) | the version | the relative path | old |
//! | outside, or ignored by git (`FileKind::Outside`) | none | the absolute path | new |
//!
//! one line, its text the line's. The draft is saved as every draft is. A thread of the
//! review whose anchor names this file (by its relative path in the same version, or for a
//! file outside by its absolute path) is drawn on its line while that line's text is still
//! the anchor's text, as a thread is in the diff, under `review-thread-<id>`; else it stays
//! among the threads elsewhere, with its notice.

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

/// The label of the control that goes back to the diff.
pub const BACK: &str = "Back to the diff";

/// The file view the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileShown {
    /// The file.
    pub file: OpenedFile,
    /// The line the reviewer is on, from 1.
    pub line: u32,
}

/// One row of the file view's list: a line of the file, a thread of the review that sits on
/// it, or one comment of that thread.
enum FileItem {
    Line(u32),
    Thread(ThreadId),
    Comment(ThreadId, CommentId),
}

/// Where each line of `text` lies, its line end not included; a text with no line has one
/// empty line.
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
    /// The file view shown; none while the diff is.
    pub fn file_view(&self) -> Option<&FileShown> {
        self.navigate.file.as_ref()
    }

    /// The text of the line `line`, from 1, of the file the file view shows.
    pub(crate) fn file_line(&self, line: u32) -> Option<&str> {
        let file = self.navigate.file.as_ref()?;
        let range = self
            .navigate
            .file_lines
            .get(line.checked_sub(1)? as usize)?;
        file.file.text.get(range.clone())
    }

    /// Asks the Navigator for the file of `location`, and shows it at the location's line.
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

    /// Shows `file` in the place of the rows, the reviewer on `line`. The place left in the
    /// diff is kept when the diff is what is shown now: a file opened from a file view
    /// leaves the diff's place as it was.
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
                offset: self.scroll.0.borrow().base_handle.offset(),
            });
        }
        self.navigate.file_lines = line_ranges(&file.text);
        let line = line.clamp(1, self.navigate.file_lines.len() as u32);
        self.navigate.file = Some(FileShown { file, line });
        self.navigate.name = None;
        // The list that led here closes, and the keys are the rows' again.
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

    /// Scrolls the file view so that the line is in view.
    fn scroll_file_to(&self, line: u32, strategy: ScrollStrategy) {
        let items = self.file_items();
        let item = items
            .iter()
            .position(|item| matches!(item, FileItem::Line(at) if *at == line))
            .unwrap_or(0);
        self.navigate.file_scroll.scroll_to_item(item, strategy);
    }

    /// What [`crate::GoBack`] does: takes a hover away if one is shown, else closes the file
    /// view and puts the diff back as it was left; with neither it does nothing.
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

    /// Closes the file view, and puts back the view, the name and the scroll of the diff
    /// that were left.
    fn leave_file_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigate.file.take().is_none() {
            return;
        }
        self.navigate.file_lines.clear();
        if let Some(left) = self.navigate.left.take() {
            self.view = left.view;
            self.navigate.name = left.name;
            self.scroll.0.borrow().base_handle.set_offset(left.offset);
        }
        self.close_list(window, cx);
        self.focus.focus(window, cx);
        self.follow_cursor();
        cx.notify();
    }

    /// The rows of the file view's list: the lines, and under each the threads of the review
    /// that sit on it.
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

    /// The threads of the review that name this file, by the last line each sits on: those
    /// whose lines still hold the text the thread was written on.
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
            // A thread of a version is on the lines of that version only; the others sit
            // where the Keeper carried them to in the files on disk.
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

    /// What [`crate::Comment`] does on a line of the file view: opens the comment box for
    /// a new thread there, with the anchor the module doc's table gives. True when the file
    /// view took the key.
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

    /// A click on the text of a line of the file view.
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

    /// The file view, drawn in the place of the rows; none while the diff is shown.
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

    /// Builds one row of the file view's list.
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

    /// [`crate::LineDown`] or [`crate::LineUp`] in the file view, `by` lines; true when
    /// the file view took it.
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
