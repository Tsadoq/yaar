//! Whose review the pane shows: the sessions of the workspace on screen that a review
//! belongs to and the review of no agent, the choice among them that the reviewer or a
//! turn end makes, and the line above the review that names them.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yaar_link::{Review, ReviewId, Session, SessionState, WorkspaceReviews};

use crate::draw::ROW_HEIGHT;
use crate::{NextReview, ReviewPane};

/// The review to show among `reviews`: the one chosen when it is there; else the review of
/// no agent while there is one; else that of the first of `sessions` that runs; else the
/// first review. `sessions` are in the order they started.
pub(crate) fn shown<'a>(
    reviews: Option<&'a WorkspaceReviews>,
    chosen: Option<&ReviewId>,
    sessions: &[Session],
) -> Option<&'a Review> {
    let reviews = &reviews?.reviews;
    let held = |id: &ReviewId| reviews.iter().find(|review| review.id == *id);
    chosen
        .and_then(held)
        .or_else(|| reviews.iter().find(|review| review.agent.is_none()))
        .or_else(|| {
            sessions
                .iter()
                .filter(|session| session.state != SessionState::NotRunning)
                .find_map(|session| session.review.as_ref().and_then(held))
        })
        .or_else(|| reviews.first())
}

impl ReviewPane {
    /// Takes up what the agents model holds of the workspace on screen: the sessions that
    /// a review belongs to. When that moves the review shown, the rows and the uncommitted
    /// changes are those of the new one.
    pub(crate) fn sessions_changed(&mut self, cx: &mut Context<Self>) {
        let now: Vec<Session> = self
            .models
            .changes
            .read(cx)
            .workspace()
            .and_then(|workspace| self.models.agents.read(cx).of(workspace))
            .map(|agents| {
                agents
                    .sessions
                    .iter()
                    .filter(|session| session.review.is_some())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if now == self.sessions {
            return;
        }
        let before = self.review().map(|review| review.id.clone());
        self.sessions = now;
        if self.review().map(|review| &review.id) == before.as_ref() {
            cx.notify();
        } else {
            self.review_moved(cx);
        }
    }

    /// The review shown is another: its threads are drawn and its uncommitted changes are
    /// asked for. Its threads may add or take rows above what the reviewer reads, so the
    /// rows are made again as reviews read anew are (`rebuild_steady`).
    pub(crate) fn review_moved(&mut self, cx: &mut Context<Self>) {
        self.rebuild_steady(cx);
        self.review_shown(cx);
    }

    /// Makes `review` the one shown, and draws the threads of it; true when that moved
    /// the review shown. The uncommitted changes of the new review are for the caller to
    /// ask for. A comment box that is open belongs to the review that was shown, so a
    /// reviewer who moves to another review leaves it, with the draft as it was saved; a
    /// turn end, which has no window to give, leaves the box as it is.
    pub(crate) fn choose_review(
        &mut self,
        review: ReviewId,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let moves = self.review().map(|shown| &shown.id) != Some(&review);
        if moves && let Some(window) = window {
            self.close_box(window, cx);
        }
        self.chosen = Some(review);
        if moves {
            // An offer to start an agent was made for the review that was shown.
            self.attaching.forget();
            self.rebuild(cx);
        }
        moves
    }

    /// Shows the review `review` at the reviewer's request, and asks for its uncommitted
    /// changes.
    pub(crate) fn show_review(
        &mut self,
        review: ReviewId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.choose_review(review, Some(window), cx) {
            self.review_shown(cx);
        }
    }

    /// The reviews the reviewer can go through, in the order the session bar names them:
    /// those of the sessions, then the review of no agent while there is one. Each with
    /// the session it belongs to, none for the review of no agent.
    fn cycle(&self) -> Vec<(ReviewId, Option<&Session>)> {
        let mut cycle: Vec<(ReviewId, Option<&Session>)> = self
            .sessions
            .iter()
            .filter_map(|session| Some((session.review.clone()?, Some(session))))
            .collect();
        let of_no_agent = self
            .reviews
            .as_ref()
            .and_then(|reviews| reviews.reviews.iter().find(|review| review.agent.is_none()));
        if let Some(review) = of_no_agent {
            cycle.push((review.id.clone(), None));
        }
        cycle
    }

    /// What a click on the entry at `index` of the session bar does: shows its review, and
    /// puts the focus on the rows, where the keys of the pane work.
    fn click_session(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        self.focus.focus(window, cx);
        if let Some((review, _)) = self.cycle().get(index).cloned() {
            self.show_review(review, window, cx);
        }
    }

    /// What [`NextReview`] does: shows the review after the one shown, the review of no
    /// agent among them, and from the last the first.
    pub(crate) fn next_review(
        &mut self,
        _: &NextReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let shown = self.review().map(|review| review.id.clone());
        let cycle = self.cycle();
        let at = cycle
            .iter()
            .position(|(review, _)| Some(review) == shown.as_ref());
        let next = at.map_or(0, |at| (at + 1) % cycle.len());
        match cycle.get(next).map(|(review, _)| review.clone()) {
            Some(review) if at != Some(next) => self.show_review(review, window, cx),
            _ => self.say("There is no other review to show", cx),
        }
    }

    /// The line that names the reviews the reviewer can show, above the tree: the sessions
    /// whose reviews there are, and the review of no agent, named "No agent", while there
    /// is one; none while there is no session. With one entry it is named under
    /// `review-session`; with several, each under `review-session-<index>`, and a click on
    /// one shows its review.
    pub(crate) fn session_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.sessions.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let cycle = self.cycle();
        let several = cycle.len() > 1;
        let shown = self.review().map(|review| &review.id);
        let mut bar = div()
            .flex_none()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_b_1()
            .border_color(theme.border);
        for (index, (review, session)) in cycle.iter().enumerate() {
            // The title is drawn in a child that carries the text in its selector, so a
            // test reads what is on screen and not a copy of it.
            let title =
                session.map_or_else(|| "No agent".to_owned(), |session| session.title.clone());
            let name = div()
                .px_2()
                .child(
                    div()
                        .debug_selector({
                            let title = title.clone();
                            move || format!("review-session-name-{index}:{title}")
                        })
                        .child(title),
                )
                .when(several && shown == Some(review), |name| {
                    name.bg(theme.secondary)
                });
            bar = bar.child(if several {
                name.debug_selector(|| format!("review-session-{index}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, _: &MouseDownEvent, window, cx| {
                            pane.click_session(index, window, cx);
                        }),
                    )
            } else {
                name.debug_selector(|| "review-session".to_owned())
            });
        }
        Some(bar)
    }
}
