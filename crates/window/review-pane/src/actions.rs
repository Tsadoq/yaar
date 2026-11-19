//! The actions of the reviewer that ask the Keeper for something that is more than a
//! save: copying the review as text, marking a file viewed, marking the change reviewed
//! and committing. None of them changes what the pane shows by itself. The Keeper tells
//! that the reviews or the changes changed, and the pane reads them again.

use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{Refusal, Reply, Request};

use crate::{Commit, CopyReview, MarkReviewed, ReviewPane, ToggleViewed};

/// The field for a commit message, while it is open, and the subscription that hears
/// the reviewer press Enter in it.
pub(crate) struct CommitField {
    pub(crate) input: Entity<InputState>,
    _heard: Subscription,
}

/// What the notice says about an answer of the Keeper that is no plain yes.
fn words(error: &AskError) -> String {
    match error {
        // The ids of a review and a workspace say nothing to the reviewer.
        AskError::Refused(Refusal::NothingToSend { .. }) => {
            "There is no unsent comment on an unresolved thread to copy".to_owned()
        }
        AskError::Refused(Refusal::NothingToCommit { .. }) => {
            "Nothing is uncommitted, so there is nothing to commit".to_owned()
        }
        error => error.to_string(),
    }
}

impl ReviewPane {
    /// What [`CopyReview`] does: asks the Keeper to copy the review as text, which sends
    /// its unsent comments, and puts the text on the clipboard when it comes. The comments
    /// show as sent when the Keeper tells that the reviews changed and the pane reads them
    /// again: the round is the Keeper's to name.
    pub(crate) fn copy_review(
        &mut self,
        _: &CopyReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        // The draft in an open box is among the comments the copy sends, and a sent
        // comment is not edited: what more is typed into the box would be refused.
        if self.open_box.is_some() {
            self.close_box(window, cx);
        }
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(Request::CopyReview { review }, cx));
        cx.spawn(async move |pane, cx| {
            let said = match asked.await {
                Ok(Reply::Copied { text, .. }) => {
                    pane.update(cx, |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    })
                    .ok();
                    return;
                }
                Ok(_) => AskError::Unexpected.to_string(),
                Err(error) => words(&error),
            };
            pane.update(cx, |pane, cx| pane.say(said, cx)).ok();
        })
        .detach();
    }

    /// What [`ToggleViewed`] does: marks the file the reviewer is in viewed, or takes the
    /// mark away when it is there.
    pub(crate) fn toggle_viewed(
        &mut self,
        _: &ToggleViewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let (Some(workspace), Some(reviews)) = (self.reviews_of.clone(), &self.reviews) else {
            return self.say("The reviews are not here yet", cx);
        };
        let Some(position) = self.position.clone() else {
            return self.say("The reviewer is in no file", cx);
        };
        let viewed = !reviews.viewed.contains(&position.path);
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::SetViewed {
                    workspace,
                    path: position.path,
                    viewed,
                },
                cx,
            )
        });
        self.report(asked, cx);
    }

    /// What [`MarkReviewed`] does: asks the Keeper to record the state of the worktree as
    /// the one reviewed. The pane shows the mark once the Keeper tells the reviews changed.
    pub(crate) fn mark_reviewed(
        &mut self,
        _: &MarkReviewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::MarkReviewed { review }, cx)
        });
        self.report(asked, cx);
    }

    /// What [`Commit`] does: opens the field for the message of the commit, with the
    /// focus. Enter in it commits, Escape closes it.
    pub(crate) fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        if let Some(open) = &self.commit_field {
            // Open already: the key brings the focus back to it and keeps its text.
            open.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.reviews_of.is_none() {
            return self.say("No workspace is on screen to commit in", cx);
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let heard = cx.subscribe_in(&input, window, |pane, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                pane.take_message(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.commit_field = Some(CommitField {
            input,
            _heard: heard,
        });
        cx.notify();
    }

    /// Enter in the field: asks the Keeper for a commit of every uncommitted change of
    /// the workspace on screen, with the text of the field, and closes the field. A field
    /// with nothing but blanks stays open: git would refuse an empty message.
    fn take_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = &self.commit_field else {
            return;
        };
        let message = field.input.read(cx).value().to_string();
        if message.trim().is_empty() {
            return self.say("Write the commit message first", cx);
        }
        let Some(workspace) = self.reviews_of.clone() else {
            return;
        };
        self.close_commit_field(window, cx);
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::Commit { workspace, message }, cx)
        });
        cx.spawn(async move |pane, cx| {
            let said = match asked.await {
                // The commit shows as the changes the Keeper tells of: nothing to say.
                Ok(Reply::Committed { .. }) => return,
                Ok(_) => AskError::Unexpected.to_string(),
                Err(error) => words(&error),
            };
            pane.update(cx, |pane, cx| pane.say(said, cx)).ok();
        })
        .detach();
    }

    /// Closes the field for a commit message, and gives the focus back to the rows.
    pub(crate) fn close_commit_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.commit_field.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    /// What the field does with Escape, which the input lets pass.
    pub(crate) fn escape_commit(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_commit_field(window, cx);
    }
}
