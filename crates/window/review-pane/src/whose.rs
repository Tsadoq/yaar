use gpui_kit::*;
use yaar_link::{Review, ReviewId, Session, SessionState, WorkspaceReviews};
use yaar_ui::pieces::Chip;

use crate::{NextReview, ReviewPane};

/// The chosen review when it is there; else the review of no agent; else that of the first
/// of `sessions` (in start order) that runs; else the first review.
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

    pub(crate) fn review_moved(&mut self, cx: &mut Context<Self>) {
        self.rebuild_steady(cx);
        self.review_shown(cx);
    }

    /// True when the review shown moved; the caller then asks for its uncommitted changes. An
    /// open comment box is closed when a window is given, as on a reviewer's move, and kept on a
    /// turn end, which has none.
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
            self.attaching.forget();
            self.rebuild(cx);
        }
        moves
    }

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

    fn click_session(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        self.focus.focus(window, cx);
        if let Some((review, _)) = self.cycle().get(index).cloned() {
            self.show_review(review, window, cx);
        }
    }

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

    pub(crate) fn session_chips(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.sessions.is_empty() {
            return Vec::new();
        }
        let cycle = self.cycle();
        let several = cycle.len() > 1;
        let shown = self.review().map(|review| &review.id);
        cycle
            .iter()
            .enumerate()
            .map(|(index, (review, session))| {
                let title =
                    session.map_or_else(|| "No agent".to_owned(), |session| session.title.clone());
                let chip = Chip::new(title.clone())
                    .selected(several && shown == Some(review))
                    .label_selector(format!("review-session-name-{index}:{title}"));
                if several {
                    chip.selector(format!("review-session-{index}"))
                        .on_click(
                            ("review-session", index),
                            cx.listener(move |pane, _: &ClickEvent, window, cx| {
                                pane.click_session(index, window, cx);
                            }),
                        )
                        .into_any_element()
                } else {
                    chip.selector("review-session").into_any_element()
                }
            })
            .collect()
    }
}
