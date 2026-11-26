use gpui_kit::*;
use yaar_link::{Review, ReviewId, Session, SessionState, WorkspaceReviews};
use yaar_ui::metrics::{SPACE_1, SPACE_2};
use yaar_ui::pieces::{Divider, ListRow, MenuTrigger, Popover, SectionLabel, State, StateDot};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

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

fn session_dot(state: SessionState) -> State {
    match state {
        SessionState::Works => State::Working,
        SessionState::WaitsForPermission => State::Permission,
        SessionState::Unreviewed => State::NeedsYou,
        SessionState::Idle => State::Ok,
        SessionState::NotRunning => State::Idle,
    }
}

struct SessionStats {
    open_threads: usize,
    total_threads: usize,
    drafts: usize,
    rounds: usize,
    reviewed: bool,
}

impl SessionStats {
    fn of(review: Option<&Review>) -> Self {
        let Some(review) = review else {
            return Self {
                open_threads: 0,
                total_threads: 0,
                drafts: 0,
                rounds: 0,
                reviewed: false,
            };
        };
        let open_threads = review.threads.iter().filter(|t| !t.resolved).count();
        let total_threads = review.threads.len();
        let drafts = review
            .threads
            .iter()
            .filter(|t| !t.resolved)
            .flat_map(|t| &t.comments)
            .filter(|c| c.round.is_none())
            .count();
        Self {
            open_threads,
            total_threads,
            drafts,
            rounds: review.rounds.len(),
            reviewed: review.reviewed.is_some(),
        }
    }

    fn is_empty_ended(&self, session: Option<&Session>) -> bool {
        session.is_some_and(|s| s.state == SessionState::NotRunning)
            && self.total_threads == 0
            && self.rounds == 0
    }

    fn past_rank(&self, session: Option<&Session>) -> u8 {
        if self.open_threads > 0 || self.drafts > 0 {
            0
        } else if self.rounds > 0 || self.total_threads > 0 || self.reviewed {
            1
        } else if session.is_none() {
            2
        } else {
            3
        }
    }

    fn summary(&self, session: Option<&Session>) -> String {
        let active = session.is_some_and(|s| s.state != SessionState::NotRunning);
        let mut parts: Vec<String> = Vec::new();
        if active && let Some(session) = session {
            let status = match session.state {
                SessionState::Works => "Working",
                SessionState::WaitsForPermission => "Needs permission",
                SessionState::Unreviewed => "Unreviewed",
                SessionState::Idle => "Idle",
                SessionState::NotRunning => "Ended",
            };
            parts.push(status.to_owned());
            if self.drafts > 0 {
                parts.push(format!(
                    "{} {}",
                    self.drafts,
                    if self.drafts == 1 { "draft" } else { "drafts" }
                ));
            } else if self.open_threads > 0 {
                parts.push(format!("{} open", self.open_threads));
            } else if self.rounds > 0 {
                parts.push(format!("Round {}", self.rounds));
            } else if self.reviewed {
                parts.push("Reviewed".to_owned());
            }
            return parts.join(" · ");
        }

        if self.rounds > 0 {
            parts.push(format!("Round {}", self.rounds));
        }
        if self.drafts > 0 {
            parts.push(format!(
                "{} {}",
                self.drafts,
                if self.drafts == 1 { "draft" } else { "drafts" }
            ));
        } else if self.open_threads > 0 {
            parts.push(format!("{} open", self.open_threads));
        } else if self.total_threads > 0 {
            parts.push(format!("{} resolved", self.total_threads));
        }
        if parts.is_empty() && self.reviewed {
            parts.push("Reviewed".to_owned());
        }
        if parts.is_empty() {
            "No comments".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

struct PickerEntry {
    index: usize,
    title: String,
    dot: State,
    summary: String,
    selected: bool,
    empty_ended: bool,
    past_rank: u8,
}

impl ReviewPane {
    pub(crate) fn close_session_picker(&mut self, cx: &mut Context<Self>) -> bool {
        if self.session_picker_open {
            self.session_picker_open = false;
            cx.notify();
            true
        } else {
            false
        }
    }

    fn toggle_session_picker(&mut self, cx: &mut Context<Self>) {
        self.session_picker_open = !self.session_picker_open;
        cx.notify();
    }

    fn toggle_show_empty_past_sessions(&mut self, cx: &mut Context<Self>) {
        self.show_empty_past_sessions = !self.show_empty_past_sessions;
        cx.notify();
    }

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
        if self.cycle().len() <= 1 {
            self.session_picker_open = false;
        }
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

    fn review_by_id(&self, id: &ReviewId) -> Option<&Review> {
        self.reviews
            .as_ref()?
            .reviews
            .iter()
            .find(|review| review.id == *id)
    }

    fn click_session(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.session_picker_open = false;
        self.acted(cx);
        self.focus.focus(window, cx);
        if let Some((review, _)) = self.cycle().get(index).cloned() {
            self.show_review(review, window, cx);
        } else {
            cx.notify();
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

    pub(crate) fn render_session_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.sessions.is_empty() {
            return None;
        }
        let cycle = self.cycle();
        if cycle.is_empty() {
            return None;
        }
        let shown = self.review().map(|review| &review.id);
        let (_, session) = cycle
            .iter()
            .find(|(review, _)| Some(review) == shown)
            .or_else(|| cycle.first())?;
        let session = *session;
        let title = session.map_or_else(|| "No agent".to_owned(), |s| s.title.clone());
        let dot = session.map_or(State::Idle, |s| session_dot(s.state));

        if cycle.len() == 1 {
            return Some(
                MenuTrigger::new("review-session", title.clone())
                    .leading_dot(dot)
                    .chevron(false)
                    .max_width(px(220.))
                    .selector("review-session")
                    .label_selector(format!("review-session-name-0:{title}"))
                    .into_any_element(),
            );
        }

        Some(
            MenuTrigger::new("review-session-picker", title.clone())
                .leading_dot(dot)
                .open(self.session_picker_open)
                .max_width(px(220.))
                .tooltip("Switch agent session review (Shift-A)")
                .selector("review-session-picker")
                .label_selector(format!("review-session-picker-name:{title}"))
                .track_bounds(self.session_picker_bounds.clone())
                .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                    pane.toggle_session_picker(cx);
                }))
                .into_any_element(),
        )
    }

    pub(crate) fn render_session_popover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.session_picker_open || self.sessions.is_empty() {
            return None;
        }
        let cycle = self.cycle();
        if cycle.len() <= 1 {
            return None;
        }
        let palette = palette(cx);
        let shown = self.review().map(|review| &review.id);
        let mut active: Vec<PickerEntry> = Vec::new();
        let mut past: Vec<PickerEntry> = Vec::new();

        for (index, (review_id, session)) in cycle.into_iter().enumerate() {
            let stats = SessionStats::of(self.review_by_id(&review_id));
            let title = session.map_or_else(|| "No agent".to_owned(), |s| s.title.clone());
            let entry = PickerEntry {
                index,
                title,
                dot: session.map_or(State::Idle, |s| session_dot(s.state)),
                summary: stats.summary(session),
                selected: shown == Some(&review_id),
                empty_ended: stats.is_empty_ended(session),
                past_rank: stats.past_rank(session),
            };
            if session.is_some_and(|s| s.state != SessionState::NotRunning) {
                active.push(entry);
            } else {
                past.push(entry);
            }
        }

        past.sort_by_key(|entry| entry.past_rank);

        let total_empty_ended = past.iter().filter(|entry| entry.empty_ended).count();
        let mut visible_past: Vec<PickerEntry> = Vec::new();
        let mut hidden_empty_count = 0usize;
        for entry in past {
            if !self.show_empty_past_sessions && entry.empty_ended && !entry.selected {
                hidden_empty_count += 1;
            } else {
                visible_past.push(entry);
            }
        }

        let draw_entry = |entry: PickerEntry, cx: &mut Context<Self>| {
            let index = entry.index;
            let title = entry.title;
            let mut row = ListRow::new(("review-session", index))
                .compact()
                .selected(entry.selected)
                .selector(format!("review-session-{index}"))
                .on_click(cx.listener(move |pane, _: &ClickEvent, window, cx| {
                    pane.click_session(index, window, cx);
                }))
                .child(StateDot::new(entry.dot))
                .child(
                    div()
                        .debug_selector({
                            let title = title.clone();
                            move || format!("review-session-name-{index}:{title}")
                        })
                        .min_w_0()
                        .truncate()
                        .typed(TypeRole::Body)
                        .text_color(palette.text)
                        .child(title),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .typed(TypeRole::Tiny)
                        .text_color(palette.text_dim)
                        .whitespace_nowrap()
                        .child(entry.summary),
                );
            if entry.selected {
                row = row.child(
                    div()
                        .flex_none()
                        .typed(TypeRole::Meta)
                        .text_color(palette.accent_text)
                        .child("\u{2713}"),
                );
            }
            row
        };

        let entity = cx.entity();
        let mut popover = Popover::new(self.session_picker_bounds.get())
            .width(px(320.))
            .max_height(px(360.))
            .selector("review-session-popover")
            .on_dismiss(move |_, cx| {
                entity.update(cx, |pane, cx| {
                    pane.close_session_picker(cx);
                });
            });

        let has_active = !active.is_empty();
        let has_past_section = !visible_past.is_empty()
            || hidden_empty_count > 0
            || (self.show_empty_past_sessions && total_empty_ended > 0);

        if has_active {
            let mut group = div().px(SPACE_2).pb(SPACE_2).flex().flex_col().gap(SPACE_1);
            for entry in active {
                group = group.child(draw_entry(entry, cx));
            }
            popover = popover
                .child(div().pt(SPACE_2).child(SectionLabel::new("Active")))
                .child(group);
        }

        if has_active && has_past_section {
            popover = popover.child(Divider::horizontal());
        }

        if has_past_section {
            let mut group = div().px(SPACE_2).pb(SPACE_2).flex().flex_col().gap(SPACE_1);
            for entry in visible_past {
                group = group.child(draw_entry(entry, cx));
            }
            if hidden_empty_count > 0 {
                let noun = if hidden_empty_count == 1 {
                    "session"
                } else {
                    "sessions"
                };
                group = group.child(
                    ListRow::new("review-session-show-empty")
                        .compact()
                        .selector("review-session-show-empty")
                        .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                            pane.toggle_show_empty_past_sessions(cx);
                        }))
                        .child(
                            div()
                                .typed(TypeRole::Small)
                                .text_color(palette.text_muted)
                                .child(format!("Show {hidden_empty_count} empty past {noun}")),
                        ),
                );
            } else if self.show_empty_past_sessions && total_empty_ended > 0 {
                group = group.child(
                    ListRow::new("review-session-show-empty")
                        .compact()
                        .selector("review-session-show-empty")
                        .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                            pane.toggle_show_empty_past_sessions(cx);
                        }))
                        .child(
                            div()
                                .typed(TypeRole::Small)
                                .text_color(palette.text_muted)
                                .child("Hide empty past sessions"),
                        ),
                );
            }
            popover = popover
                .child(div().pt(SPACE_2).child(SectionLabel::new("Past reviews")))
                .child(group);
        }

        Some(popover.into_any_element())
    }
}
