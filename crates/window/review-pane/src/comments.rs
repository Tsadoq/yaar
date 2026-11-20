//! What the pane does with the reviews of the workspace on screen: it reads them, keeps
//! them in step with its own saves, and lets the reviewer write, edit, delete and walk
//! through the comments of threads.

use gpui_kit::component::input::{Escape, InputEvent, TextareaState};
use gpui_kit::*;
use yaar_diff::Side;
use yaar_frame::AskError;
use yaar_link::{
    Anchor, CommentId, DraftOn, Fact, Lines, Reply, Request, Thread, ThreadId, WorkspaceId,
};

use crate::saves::{Save, apply, new_id};
use crate::threads::{link_side, pane_side, path_in};
use crate::tree::Source;
use crate::{
    Comment, CommentBox, DeleteDraft, EditDraft, NextThread, PaneRow, Position, PreviousThread,
    ReviewPane, View,
};

/// How many lines of text a comment box grows to before it scrolls.
const BOX_ROWS: usize = 6;

/// A comment box that is open: what it says to the pane, the field that holds its text,
/// and the subscription that hears the field change.
pub(crate) struct OpenBox {
    pub(crate) view: CommentBox,
    pub(crate) input: Entity<TextareaState>,
    _heard: Subscription,
}

/// A thread of the review as the rows have it, in their order.
struct ThreadStop {
    thread: ThreadId,
    /// The index of the row of the diff the thread follows; none for a thread among
    /// those elsewhere, which has no line to stand on.
    line_row: Option<usize>,
}

impl ReviewPane {
    /// Takes up the workspace on screen: when it is another than the one the reviews
    /// belong to, they are let go, with the box that was open on them. Asks the Keeper for
    /// them whenever the changes of that workspace have a new answer or a refusal, which
    /// covers a workspace that just came on screen, one whose worktree is gone, and the
    /// window coming back to the front.
    pub(crate) fn follow_reviews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changes = self.models.changes.read(cx);
        let workspace = changes.workspace().cloned();
        let answered =
            !changes.is_loading() && (changes.changes().is_some() || changes.refusal().is_some());
        if workspace != self.reviews_of {
            self.reviews_of = workspace.clone();
            self.reviews = None;
            self.chosen = None;
            self.saves.clear();
            self.reading_reviews = None;
            self.notice = None;
            if self.open_box.is_some() {
                self.close_box(window, cx);
            }
            // A commit is of the workspace the field was opened on.
            if self.commit_field.is_some() {
                self.close_commit_field(window, cx);
            }
            self.rebuild(cx);
        }
        if let (true, Some(workspace)) = (answered, workspace) {
            self.read_reviews(workspace, cx);
        }
    }

    /// What the pane does with a fact of the Keeper: the reviews of the workspace on
    /// screen changed, and are read again.
    pub(crate) fn told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        match fact {
            Fact::ReviewsChanged { workspace } if self.reviews_of.as_ref() == Some(workspace) => {
                self.read_reviews(workspace.clone(), cx);
            }
            // The history grows when a commit is made or a branch is switched.
            Fact::Workspaces(_) => self.read_history(cx),
            // What is uncommitted changes with the files, and with the turn that ended.
            Fact::ChangesChanged { workspace } if self.tree.of.as_ref() == Some(workspace) => {
                self.read_uncommitted(cx);
            }
            Fact::TurnEnded {
                workspace,
                session,
                state,
                since,
                ..
            } if self.tree.of.as_ref() == Some(workspace) => {
                self.read_history(cx);
                // A turn whose state was not recorded has no change to select.
                if let Some(state) = state {
                    self.turn_ended(session, state.clone(), since.clone(), cx);
                }
                self.read_uncommitted(cx);
            }
            _ => {}
        }
        // Flow hears every fact too: the timeline first, so that the live diff and follow
        // find in it the step they are told of.
        self.flow_told(fact, cx);
        self.live_told(fact, cx);
        self.follow_told(fact, cx);
    }

    /// Asks the Keeper for the reviews of `workspace`. Only the answer to the last
    /// question is taken.
    fn read_reviews(&mut self, workspace: WorkspaceId, cx: &mut Context<Self>) {
        // The Keeper answers the requests of reviews of one Window in the order they were
        // sent, so every save sent before this question is in its answer.
        let asked_after = self.saved;
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::GetReviews {
                    workspace: workspace.clone(),
                },
                cx,
            )
        });
        self.reading_reviews = Some(cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                pane.reviews_answered(&workspace, asked_after, answer, cx)
            })
            .ok();
        }));
    }

    /// Shows an answer of the Keeper with the saves sent after it was asked for put over
    /// it. A refusal shows no review and no notice: what the pane has stays, and the pane
    /// works on.
    fn reviews_answered(
        &mut self,
        workspace: &WorkspaceId,
        asked_after: u64,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        if self.reviews_of.as_ref() != Some(workspace) {
            return;
        }
        let Ok(Reply::Reviews(mut shown)) = answer else {
            return;
        };
        if shown.workspace != *workspace {
            return;
        }
        // The saves sent before the question are in the answer; the others are not yet.
        self.saves.retain(|save| save.seq > asked_after);
        for save in &self.saves {
            apply(&mut shown, save);
        }
        self.attaching.reviews_read(&shown);
        self.reviews = Some(shown);
        // Read anew, the reviews may add or take rows above what the reviewer reads.
        self.rebuild_steady(cx);
        self.review_shown(cx);
    }

    /// Says something under the rows.
    pub(crate) fn say(&mut self, words: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(words.into());
        cx.notify();
    }

    /// Called first by every action of the reviewer: what was said is gone, and the
    /// reviewer at work in the pane pauses follow ([`ReviewPane::touched`]). A key bound to
    /// an action reaches the pane only as that action, never as a key event, so this is
    /// where a key of the pane pauses follow. The controls of the views and of follow call
    /// [`ReviewPane::steered`] instead.
    pub(crate) fn acted(&mut self, cx: &mut Context<Self>) {
        self.steered(cx);
        self.touched(cx);
    }

    /// Called first by the four controls of the views and of follow, [`crate::ToggleView`],
    /// [`crate::ToggleFlowForm`], [`crate::ToggleFollow`] and [`crate::ResumeFollow`], by key
    /// or by a click on the bar: what was said is gone, and follow is not paused, since
    /// they choose what Flow shows rather than read it. `ToggleFlowForm` and
    /// `ResumeFollow` act only while Flow is on screen; in Changes they do not get this
    /// far, and leave the notice as it is.
    pub(crate) fn steered(&mut self, cx: &mut Context<Self>) {
        if self.notice.take().is_some() {
            cx.notify();
        }
    }

    /// Where the reviewer's line would be anchored: the version it is read in, the path
    /// it has there, its side, its line and its text.
    fn anchor_at(&self, position: &Position) -> Result<Anchor, String> {
        let file = self
            .by_path
            .get(&position.path)
            .map(|&index| &self.files[index].file)
            .ok_or("The line is of a file the pane does not hold")?;
        let text = self
            .text(position)
            .ok_or("The text of the line is not here")?;
        // The old side is read in the version the change is counted from, which the
        // Keeper keeps; the new side is in the version the change leads to, or on disk,
        // and the Keeper records it.
        let tree = match (position.side, &self.listed) {
            (Side::Old, Some(Source::Versions { old, .. })) => Some(
                old.clone()
                    .ok_or("There is no version the old lines could be counted from")?,
            ),
            (Side::New, Some(Source::Versions { new, .. })) => new.clone(),
            (Side::Old, _) => Some(
                self.changes
                    .as_ref()
                    .and_then(|changes| changes.counted_from.clone())
                    .ok_or("There is no commit the old lines could be counted from")?,
            ),
            (Side::New, _) => None,
        };
        let line = position.line + 1;
        Ok(Anchor {
            tree,
            path: path_in(file, position.side).to_owned(),
            side: link_side(position.side),
            lines: Lines {
                first: line,
                last: line,
            },
            text,
        })
    }

    /// The threads of the review that sit on the line the reviewer is on, those of the
    /// reviewer's own side first. An unchanged line is one row with a line on each side,
    /// and a thread of either side sits there: the reviewer stands on it by one side
    /// whichever the thread is on.
    fn threads_on(&self, position: &Position) -> Vec<&Thread> {
        let Some(&index) = self.by_path.get(&position.path) else {
            return Vec::new();
        };
        let file = &self.files[index].file;
        let on_both_sides = match self.position_row.and_then(|row| self.rows.get(row)) {
            Some(PaneRow::Diff { row, .. }) if matches!(row, yaar_diff::Row::Context { .. }) => {
                Some(row)
            }
            _ => None,
        };
        let Some(review) = self.review() else {
            return Vec::new();
        };
        let mut threads: Vec<&Thread> = review
            .threads
            .iter()
            .filter(|thread| {
                let side = pane_side(thread.anchor.side);
                let line = match on_both_sides {
                    Some(row) => row.line(side),
                    None => (side == position.side).then_some(position.line),
                };
                path_in(file, side) == thread.anchor.path
                    && line.is_some_and(|line| thread.anchor.lines.last == line + 1)
            })
            .collect();
        // Stable: threads of one side keep the order the Keeper holds them in.
        threads.sort_by_key(|thread| pane_side(thread.anchor.side) != position.side);
        threads
    }

    /// The thread on the reviewer's line with the draft of the reviewer in it, if any.
    fn draft_on(&self, position: &Position) -> Option<(&Thread, CommentId)> {
        self.threads_on(position).into_iter().find_map(|thread| {
            let draft = thread
                .comments
                .iter()
                .find(|comment| comment.round.is_none())?;
            Some((thread, draft.id.clone()))
        })
    }

    /// What [`Comment`] does: opens a comment box for a new thread on the reviewer's line.
    pub(crate) fn comment(&mut self, _: &Comment, window: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        // In Flow the reviewer's line is one of the live diff.
        if self.view == View::Flow {
            return self.comment_in_flow(window, cx);
        }
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        let Some(position) = self.position.clone() else {
            return self.say("The reviewer is on no line", cx);
        };
        let anchor = match self.anchor_at(&position) {
            Ok(anchor) => anchor,
            Err(why) => return self.say(why, cx),
        };
        let (Ok(thread), Ok(comment)) = (new_id(), new_id()) else {
            return self.say("An id for the comment could not be drawn", cx);
        };
        self.open_box(
            CommentBox {
                review,
                thread: ThreadId::new(thread),
                comment: CommentId::new(comment),
                anchor: Some(anchor),
                at: position,
                text: String::new(),
            },
            window,
            cx,
        );
    }

    /// What [`EditDraft`] does: opens the reviewer's draft in the thread on the
    /// reviewer's line.
    pub(crate) fn edit_draft(
        &mut self,
        _: &EditDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(position) = self.position.clone() else {
            return self.say("The reviewer is on no line", cx);
        };
        let Some((thread, comment)) = self.draft_on(&position) else {
            return self.say("There is no draft of yours on this line to edit", cx);
        };
        let text = thread
            .comments
            .iter()
            .find(|candidate| candidate.id == comment)
            .map(|candidate| candidate.text.clone())
            .unwrap_or_default();
        let (thread, review) = (thread.id.clone(), self.review().map(|r| r.id.clone()));
        let Some(review) = review else { return };
        self.open_box(
            CommentBox {
                review,
                thread,
                comment,
                anchor: None,
                at: position,
                text,
            },
            window,
            cx,
        );
    }

    /// What [`crate::Reply`] does: opens a comment box for a reply in the thread on the
    /// reviewer's line, once its last comment was sent and it holds no draft.
    pub(crate) fn reply(&mut self, _: &crate::Reply, window: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        let Some(position) = self.position.clone() else {
            return self.say("The reviewer is on no line", cx);
        };
        let threads = self.threads_on(&position);
        if threads.is_empty() {
            return self.say("There is no thread on this line to reply in", cx);
        }
        let holds_draft = |thread: &Thread| {
            thread
                .comments
                .iter()
                .any(|comment| comment.round.is_none())
        };
        // A thread that holds a draft takes no second one, and one with nothing sent
        // has nothing to reply to: the reply goes to the first thread that can have it.
        let Some(thread) = threads
            .iter()
            .find(|thread| !holds_draft(thread) && !thread.comments.is_empty())
        else {
            return if threads.iter().any(|thread| holds_draft(thread)) {
                self.say("This thread holds a draft already: edit it", cx)
            } else {
                self.say("Nothing was sent in this thread yet", cx)
            };
        };
        let thread = thread.id.clone();
        let (Some(review), Ok(comment)) = (self.review().map(|r| r.id.clone()), new_id()) else {
            return self.say("An id for the comment could not be drawn", cx);
        };
        self.open_box(
            CommentBox {
                review,
                thread,
                comment: CommentId::new(comment),
                anchor: None,
                at: position,
                text: String::new(),
            },
            window,
            cx,
        );
    }

    /// What [`DeleteDraft`] does: asks the Keeper to delete the reviewer's draft in the
    /// thread on the reviewer's line. The Keeper tells that the reviews changed, and the
    /// pane reads them again.
    pub(crate) fn delete_draft(
        &mut self,
        _: &DeleteDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(position) = self.position.clone() else {
            return self.say("The reviewer is on no line", cx);
        };
        let Some((_, comment)) = self.draft_on(&position) else {
            return self.say("There is no draft of yours on this line to delete", cx);
        };
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return;
        };
        if self
            .open_box
            .as_ref()
            .is_some_and(|open| open.view.comment == comment)
        {
            self.close_box(window, cx);
        }
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::DeleteDraft { review, comment }, cx)
        });
        self.report(asked, cx);
    }

    /// Says why a request of the reviewer was not done, when the Keeper's answer is not
    /// a plain yes.
    pub(crate) fn report(&mut self, asked: Task<Result<Reply, AskError>>, cx: &mut Context<Self>) {
        cx.spawn(async move |pane, cx| {
            let words = match asked.await {
                Ok(Reply::Done) => return,
                Ok(_) => AskError::Unexpected.to_string(),
                Err(error) => error.to_string(),
            };
            pane.update(cx, |pane, cx| pane.say(words, cx)).ok();
        })
        .detach();
    }

    /// The threads of the review in the order of the rows, each with the row of the line
    /// it follows.
    fn thread_stops(&self) -> Vec<ThreadStop> {
        let mut stops = Vec::new();
        let mut line_row = None;
        let mut elsewhere = false;
        for (index, row) in self.rows.iter().enumerate() {
            match row {
                PaneRow::Diff { .. } => line_row = Some(index),
                PaneRow::ThreadsElsewhere => elsewhere = true,
                PaneRow::Thread { thread } => stops.push(ThreadStop {
                    thread: thread.clone(),
                    line_row: if elsewhere { None } else { line_row },
                }),
                _ => {}
            }
        }
        stops
    }

    /// Puts the reviewer on the line of a thread that is shown on one.
    fn go_to_thread(&mut self, stop: &ThreadStop, cx: &mut Context<Self>) {
        let (Some(index), Some(side)) = (
            stop.line_row,
            self.review()
                .and_then(|review| review.threads.iter().find(|t| t.id == stop.thread))
                .map(|thread| pane_side(thread.anchor.side)),
        ) else {
            return;
        };
        // An unchanged line is on both sides and the reviewer is on it by its new one.
        let side = match self.rows.get(index) {
            Some(PaneRow::Diff {
                row: yaar_diff::Row::Context { .. },
                ..
            }) => Side::New,
            _ => side,
        };
        self.go_to(index, side, cx);
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
    }

    /// What [`NextThread`] does: puts the reviewer on the line of the first thread whose
    /// line comes after theirs.
    pub(crate) fn next_thread(&mut self, _: &NextThread, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        let from = self.position_row;
        let stops = self.thread_stops();
        let next = stops.iter().find(|stop| match stop.line_row {
            Some(row) => from.is_none_or(|from| row > from),
            None => true,
        });
        match next {
            Some(
                stop @ ThreadStop {
                    line_row: Some(_), ..
                },
            ) => self.go_to_thread(stop, cx),
            Some(_) => self.say(
                "The next threads are under Threads elsewhere, on no line",
                cx,
            ),
            None => self.say("There is no thread after this line", cx),
        }
    }

    /// What [`PreviousThread`] does: puts the reviewer on the line of the last thread
    /// whose line comes before theirs.
    pub(crate) fn previous_thread(
        &mut self,
        _: &PreviousThread,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let from = self.position_row;
        let stops = self.thread_stops();
        let previous = stops
            .iter()
            .rev()
            .find(|stop| matches!((stop.line_row, from), (Some(row), Some(from)) if row < from));
        match previous {
            Some(stop) => self.go_to_thread(stop, cx),
            None => self.say("There is no thread before this line", cx),
        }
    }

    /// Opens a comment box, with the focus, in the place of the one that is open.
    pub(crate) fn open_box(
        &mut self,
        view: CommentBox,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = view.text.clone();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, BOX_ROWS)
                // Enter is the end of the comment, as in tuicr: Shift-Enter is a new line.
                .submit_on_enter(true)
        });
        input.update(cx, |input, cx| {
            if !text.is_empty() {
                input.set_value(text.clone(), window, cx);
                // The next key goes after the draft, not before it.
                input.set_selected_range(text.len()..text.len(), cx);
            }
            input.focus(window, cx);
        });
        let heard =
            cx.subscribe_in(
                &input,
                window,
                |pane, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => pane.typed(cx),
                    InputEvent::PressEnter { shift: false, .. } => pane.close_box(window, cx),
                    _ => {}
                },
            );
        self.open_box = Some(OpenBox {
            view,
            input,
            _heard: heard,
        });
        cx.notify();
    }

    /// Closes the comment box, which keeps its draft, and gives the focus back to the
    /// rows, where the keys of the pane work.
    pub(crate) fn close_box(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_box.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    /// What the box does with Escape, which the field lets pass.
    pub(crate) fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        self.close_box(window, cx);
    }

    /// The text of the box changed: it is saved at once, with the whole text.
    fn typed(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open_box else {
            return;
        };
        let text = open.input.read(cx).value().to_string();
        if text == open.view.text {
            return;
        }
        open.view.text.clone_from(&text);
        let view = open.view.clone();
        self.save(&view, text, cx);
    }

    /// Asks the Keeper to keep a draft as it is now, without waiting for the answer to
    /// the one before, and puts it into the reviews the pane shows.
    fn save(&mut self, view: &CommentBox, text: String, cx: &mut Context<Self>) {
        let on = match &view.anchor {
            Some(anchor) => DraftOn::NewThread {
                anchor: anchor.clone(),
            },
            None => DraftOn::Thread,
        };
        let request = Request::SaveDraft {
            review: view.review.clone(),
            thread: view.thread.clone(),
            comment: view.comment.clone(),
            on,
            text: text.clone(),
        };
        // Asked here, so that the saves reach the Keeper in the order they were typed.
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        self.saved += 1;
        let save = Save {
            seq: self.saved,
            review: view.review.clone(),
            thread: view.thread.clone(),
            comment: view.comment.clone(),
            anchor: view.anchor.clone(),
            text,
        };
        let added = self
            .reviews
            .as_mut()
            .is_some_and(|reviews| apply(reviews, &save));
        // Each draft is held once, as it was last saved: the whole text is in every save.
        match self
            .saves
            .iter_mut()
            .find(|held| held.comment == save.comment)
        {
            Some(held) => {
                held.seq = save.seq;
                held.text = save.text;
            }
            None => self.saves.push(save),
        }
        if added {
            self.rebuild(cx);
        } else {
            cx.notify();
        }
        self.report(asked, cx);
    }
}
