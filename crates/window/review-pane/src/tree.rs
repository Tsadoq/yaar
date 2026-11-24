//! The history of the worktree above the files, and which entry or range of entries the files are
//! the change of.

use std::ops::Range;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yaar_link::{
    ChangedFile, CreditedFile, History, HistoryEntry, Reply, Request, ReviewId, WorkspaceId,
};

use crate::actions::words;
use crate::draw::ROW_HEIGHT;
use crate::turns::Offer;
use crate::{
    ChangeShown, EXPIRED_TURN, FocusHistory, FocusRows, HISTORY_CONTEXT, HistoryDown, HistoryUp,
    MarkChange, ReviewPane, ShowChange,
};

const ENTRIES_IN_VIEW: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    Comparison,
    /// The two versions the Keeper names: the old one (none where nothing came before), the new one
    /// (none for the files on disk).
    Versions {
        old: Option<String>,
        new: Option<String>,
    },
}

pub(crate) struct Credits {
    pub(crate) review: ReviewId,
    pub(crate) files: Vec<CreditedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListedFor {
    /// No review: a change since a round, a turn end of a session with no review here, an entry,
    /// a range, the files on disk. It stays as it is when another review is shown.
    NoReview,
    Review(ReviewId),
    NoAgentReview,
}

impl ListedFor {
    fn review(&self) -> Option<&ReviewId> {
        match self {
            Self::Review(review) => Some(review),
            Self::NoReview | Self::NoAgentReview => None,
        }
    }
}

pub(crate) struct Tree {
    pub(crate) of: Option<WorkspaceId>,
    pub(crate) history: Option<History>,
    reading_history: Option<Task<()>>,
    pub(crate) uncommitted: Option<Vec<CreditedFile>>,
    uncommitted_for: Option<ReviewId>,
    reading_uncommitted: Option<Task<()>>,
    pub(crate) cursor: Option<usize>,
    pub(crate) marked: Vec<usize>,
    pub(crate) focus: FocusHandle,
    scroll: UniformListScrollHandle,
    pub(crate) credits: Option<Credits>,
    pub(crate) shown: ChangeShown,
    /// Whether `shown` was picked by the pane for a window with no changes yet, not selected by the
    /// reviewer.
    pub(crate) first_picture: bool,
    pub(crate) wanted: Source,
    /// Kept apart from `credits`, which hold the review of the list last answered, not of the
    /// request on its way.
    pub(crate) wanted_for: ListedFor,
    pub(crate) listing: Option<Task<()>>,
    pub(crate) offer: Option<Offer>,
}

impl Tree {
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        Self {
            of: None,
            history: None,
            reading_history: None,
            uncommitted: None,
            uncommitted_for: None,
            reading_uncommitted: None,
            cursor: None,
            marked: Vec::new(),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            credits: None,
            shown: ChangeShown::Comparison,
            first_picture: false,
            wanted: Source::Comparison,
            wanted_for: ListedFor::NoReview,
            listing: None,
            offer: None,
        }
    }
}

pub(crate) fn version_of(entry: &HistoryEntry) -> Option<&str> {
    match entry {
        HistoryEntry::Commit(commit) => Some(&commit.id),
        HistoryEntry::Turn { state, .. } => state.as_deref(),
        HistoryEntry::ExpiredTurn { .. } => None,
    }
}

fn before(history: &History, first: usize) -> Option<String> {
    history.entries[..first]
        .iter()
        .rev()
        .find_map(version_of)
        .or(history.start.as_deref())
        .map(str::to_owned)
}

/// The newest commit, or the history's start where it holds none; the uncommitted changes are
/// counted from it.
pub(crate) fn checked_out(history: &History) -> Option<String> {
    history
        .entries
        .iter()
        .rev()
        .find_map(|entry| match entry {
            HistoryEntry::Commit(commit) => Some(commit.id.clone()),
            HistoryEntry::Turn { .. } | HistoryEntry::ExpiredTurn { .. } => None,
        })
        .or_else(|| history.start.clone())
}

impl ReviewPane {
    fn tree_len(&self) -> usize {
        self.tree
            .history
            .as_ref()
            .map_or(0, |history| history.entries.len() + 1)
    }

    pub(crate) fn has_tree(&self) -> bool {
        self.tree.history.as_ref().is_some_and(|history| {
            !history.entries.is_empty()
                || self
                    .tree
                    .uncommitted
                    .as_ref()
                    .is_some_and(|listed| !listed.is_empty())
        })
    }

    fn selectable(&self, index: usize) -> bool {
        self.tree
            .history
            .as_ref()
            .and_then(|history| history.entries.get(index))
            .and_then(version_of)
            .is_some()
    }

    pub(crate) fn follow_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.models.changes.read(cx).workspace().cloned();
        if workspace == self.tree.of {
            return;
        }
        if self.tree.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
        self.tree = Tree::new(cx);
        self.tree.of = workspace;
        self.listed = None;
        self.read_history(cx);
        cx.notify();
    }

    pub(crate) fn read_history(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.tree.of.clone() else {
            return;
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::GetHistory {
                    workspace: workspace.clone(),
                },
                cx,
            )
        });
        self.tree.reading_history = Some(cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                if let Ok(Reply::History(history)) = answer
                    && history.workspace == workspace
                    && pane.tree.of.as_ref() == Some(&workspace)
                {
                    pane.history_answered(history, cx);
                }
            })
            .ok();
        }));
    }

    fn history_answered(&mut self, history: History, cx: &mut Context<Self>) {
        // An answer that tells nothing new (the second of two questions asked close together)
        // changes nothing.
        if self.tree.history.as_ref() == Some(&history) {
            return;
        }
        let entries = history.entries.len();
        self.tree.history = Some(history);
        self.tree.marked.retain(|&index| index < entries);
        if let Some(cursor) = self.tree.cursor {
            self.tree.cursor = Some(cursor.min(entries));
        }
        match self.tree.shown.clone() {
            ChangeShown::Entries { first, last } => {
                if !self.select_entries(first, last, !self.tree.first_picture, cx) {
                    self.back_to_comparison(cx);
                }
            }
            ChangeShown::Uncommitted => {
                let base = self.tree.history.as_ref().and_then(checked_out);
                if self.tree.wanted
                    != (Source::Versions {
                        old: base,
                        new: None,
                    })
                {
                    self.select_uncommitted(cx);
                }
            }
            ChangeShown::Comparison => {
                // The history is read from the Keeper's records, the changes need git: show the
                // last recorded turn meanwhile.
                if self.changes.is_none()
                    && let Some(last) = self.tree.history.as_ref().and_then(|history| {
                        history.entries.iter().rposition(|entry| {
                            matches!(entry, HistoryEntry::Turn { state: Some(_), .. })
                        })
                    })
                {
                    self.select_entries(last, last, false, cx);
                }
            }
            ChangeShown::Since { .. } => {}
        }
        self.flow_history(cx);
        self.live_history(cx);
        cx.notify();
    }

    pub(crate) fn read_uncommitted(&mut self, cx: &mut Context<Self>) {
        let (Some(workspace), Some(review)) = (
            self.tree.of.clone(),
            self.review().map(|review| review.id.clone()),
        ) else {
            return;
        };
        self.tree.uncommitted_for = Some(review.clone());
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::GetUncommitted {
                    workspace: workspace.clone(),
                    review: Some(review.clone()),
                },
                cx,
            )
        });
        self.tree.reading_uncommitted = Some(cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                if let Ok(Reply::Uncommitted(listed)) = answer
                    && pane.tree.of.as_ref() == Some(&workspace)
                    && pane.tree.uncommitted_for.as_ref() == Some(&review)
                {
                    pane.uncommitted_answered(listed, cx);
                }
            })
            .ok();
        }));
    }

    pub(crate) fn review_shown(&mut self, cx: &mut Context<Self>) {
        let now = self.review().map(|review| &review.id);
        if now.is_some() && self.tree.uncommitted_for.as_ref() != now {
            self.read_uncommitted(cx);
        }
        self.list_again_for_review_shown(cx);
    }

    /// A change listed for a review is that review's own list, the review of no agent included:
    /// when another review is shown it is listed again. A change listed with no review stays.
    fn list_again_for_review_shown(&mut self, cx: &mut Context<Self>) {
        let (ChangeShown::Since { from, to }, Some(review)) =
            (self.tree.shown.clone(), self.review())
        else {
            return;
        };
        if self.tree.wanted_for == ListedFor::NoReview {
            return;
        }
        let now = if review.agent.is_some() {
            ListedFor::Review(review.id.clone())
        } else {
            ListedFor::NoAgentReview
        };
        if now != self.tree.wanted_for {
            self.select_since_for(from, to, now, cx);
        }
    }

    fn uncommitted_answered(&mut self, listed: Vec<CreditedFile>, cx: &mut Context<Self>) {
        self.tree.uncommitted = Some(listed);
        if self.tree.shown == ChangeShown::Uncommitted {
            self.select_uncommitted(cx);
        }
        cx.notify();
    }

    /// False when the history is not here, an entry does not exist, or an end of the range holds no
    /// version.
    pub(crate) fn select_entries(
        &mut self,
        first: usize,
        last: usize,
        by_reviewer: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(history) = &self.tree.history else {
            return false;
        };
        if first > last || !self.selectable(first) || !self.selectable(last) {
            return false;
        }
        let new = history
            .entries
            .get(last)
            .and_then(version_of)
            .map(str::to_owned);
        let old = before(history, first);
        let workspace = history.workspace.clone();
        self.tree.first_picture = !by_reviewer;
        self.tree.shown = ChangeShown::Entries { first, last };
        cx.notify();
        self.list_change(workspace, old, new, ListedFor::NoReview, cx);
        true
    }

    pub(crate) fn select_since(
        &mut self,
        from: Option<String>,
        to: String,
        cx: &mut Context<Self>,
    ) {
        self.select_since_for(from, to, ListedFor::NoReview, cx);
    }

    /// For an agent session's review only the files it shows, each with its credits; for the review
    /// of no agent, every file with none.
    pub(crate) fn select_since_for(
        &mut self,
        from: Option<String>,
        to: String,
        listed_for: ListedFor,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.tree.of.clone() else {
            return;
        };
        self.tree.first_picture = false;
        self.tree.marked.clear();
        self.tree.shown = ChangeShown::Since {
            from: from.clone(),
            to: to.clone(),
        };
        cx.notify();
        self.list_change(workspace, from, Some(to), listed_for, cx);
    }

    fn list_change(
        &mut self,
        workspace: WorkspaceId,
        old: Option<String>,
        new: Option<String>,
        listed_for: ListedFor,
        cx: &mut Context<Self>,
    ) {
        let source = Source::Versions {
            old: old.clone(),
            new: new.clone(),
        };
        // `CompareForReview` names both versions: the files on disk are listed for no review.
        let listed_for = match listed_for {
            ListedFor::Review(_) if new.is_none() => ListedFor::NoReview,
            other => other,
        };
        let review = listed_for.review().cloned();
        // The same two versions for another review are another list, even while the earlier request
        // is on its way. A list for a review the Keeper refused is asked for again.
        let credited_for = self.tree.credits.as_ref().map(|credits| &credits.review);
        if self.tree.wanted == source
            && self.listed.as_ref() == Some(&source)
            && self.tree.wanted_for == listed_for
            && (review.is_none() || credited_for == review.as_ref())
        {
            return;
        }
        self.tree.wanted = source.clone();
        self.tree.wanted_for = listed_for;
        self.tree.listing = None;
        let Some(from) = old else {
            self.say(
                "Nothing came before this change, so its files cannot be listed",
                cx,
            );
            self.tree.credits = None;
            self.show_files(Vec::new(), source, cx);
            return;
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                match (&review, new.clone()) {
                    (Some(review), Some(to)) => Request::CompareForReview {
                        workspace,
                        review: review.clone(),
                        from,
                        to,
                    },
                    _ => Request::CompareVersions {
                        workspace,
                        from,
                        to: new,
                    },
                },
                cx,
            )
        });
        // A selection that replaces this one drops the task, and its answer with it.
        self.tree.listing = Some(cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                let (files, credits) = match (answer, review) {
                    (Ok(Reply::Compared(files)), None) => (files, None),
                    (Ok(Reply::ComparedForReview(credited)), Some(review)) => (
                        credited
                            .iter()
                            .map(|credited| credited.file.clone())
                            .collect(),
                        Some(Credits {
                            review,
                            files: credited,
                        }),
                    ),
                    (Ok(_), _) => {
                        pane.say(yaar_frame::AskError::Unexpected.to_string(), cx);
                        (Vec::new(), None)
                    }
                    (Err(error), _) => {
                        pane.say(words(&error), cx);
                        (Vec::new(), None)
                    }
                };
                pane.tree.credits = credits;
                pane.show_files(files, source, cx);
            })
            .ok();
        }));
    }

    pub(crate) fn select_uncommitted(&mut self, cx: &mut Context<Self>) {
        let source = Source::Versions {
            old: self.tree.history.as_ref().and_then(checked_out),
            new: None,
        };
        self.tree.shown = ChangeShown::Uncommitted;
        self.tree.credits = None;
        self.tree.first_picture = false;
        self.tree.wanted = source.clone();
        self.tree.wanted_for = ListedFor::NoReview;
        self.tree.listing = None;
        let files: Vec<ChangedFile> = self
            .tree
            .uncommitted
            .iter()
            .flatten()
            .map(|credited| credited.file.clone())
            .collect();
        self.show_files(files, source, cx);
        cx.notify();
    }

    pub(crate) fn back_to_comparison(&mut self, cx: &mut Context<Self>) {
        self.tree.shown = ChangeShown::Comparison;
        self.tree.credits = None;
        self.tree.first_picture = false;
        self.tree.wanted = Source::Comparison;
        self.tree.wanted_for = ListedFor::NoReview;
        self.tree.listing = None;
        match self.changes.clone() {
            Some(changes) => self.show(changes, cx),
            None => {
                self.listed = None;
                self.show_files(Vec::new(), Source::Comparison, cx);
                self.listed = None;
            }
        }
    }

    pub(crate) fn focus_history(
        &mut self,
        _: &FocusHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        if !self.has_tree() {
            return self.say("There is no history of changes to show", cx);
        }
        self.tree.cursor.get_or_insert(0);
        self.tree.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn focus_rows(
        &mut self,
        _: &FocusRows,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn step_cursor(&mut self, by: isize, cx: &mut Context<Self>) {
        self.acted(cx);
        let Some(last) = self.tree_len().checked_sub(1) else {
            return;
        };
        let now = self.tree.cursor.unwrap_or(0);
        let next = now.saturating_add_signed(by).min(last);
        self.tree.cursor = Some(next);
        self.tree
            .scroll
            .scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(crate) fn history_down(&mut self, _: &HistoryDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step_cursor(1, cx);
    }

    pub(crate) fn history_up(&mut self, _: &HistoryUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step_cursor(-1, cx);
    }

    pub(crate) fn mark_change(&mut self, _: &MarkChange, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        let Some(entry) = self.tree.cursor.filter(|&entry| self.selectable(entry)) else {
            return;
        };
        match self.tree.marked.binary_search(&entry) {
            Ok(at) => drop(self.tree.marked.remove(at)),
            Err(at) => self.tree.marked.insert(at, entry),
        }
        cx.notify();
    }

    pub(crate) fn show_change(&mut self, _: &ShowChange, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        if let (Some(&first), Some(&last)) = (self.tree.marked.first(), self.tree.marked.last()) {
            self.tree.offer = None;
            self.tree.marked.clear();
            self.select_entries(first, last, true, cx);
            return;
        }
        let Some(entry) = self.tree.cursor else {
            return;
        };
        self.select(entry, false, cx);
    }

    fn select(&mut self, index: usize, extend: bool, cx: &mut Context<Self>) {
        self.tree.offer = None;
        self.tree.marked.clear();
        let entries = self
            .tree
            .history
            .as_ref()
            .map_or(0, |history| history.entries.len());
        if index >= entries {
            return self.select_uncommitted(cx);
        }
        let (first, last) = match self.tree.shown {
            ChangeShown::Entries { first, last } if extend => (first.min(index), last.max(index)),
            _ => (index, index),
        };
        self.select_entries(first, last, true, cx);
    }

    fn click_entry(
        &mut self,
        index: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.tree.focus.focus(window, cx);
        self.tree.cursor = Some(index);
        if index < self.tree_len().saturating_sub(1) && !self.selectable(index) {
            return cx.notify();
        }
        self.select(index, shift, cx);
    }

    pub(crate) fn tree_view(&self, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        if !self.has_tree() {
            if self.tree.focus.is_focused(window) {
                cx.defer_in(window, |pane, window, cx| pane.focus.focus(window, cx));
            }
            return None;
        }
        let theme = cx.theme();
        let shown = self.tree_len().min(ENTRIES_IN_VIEW);
        let list = uniform_list(
            "review-history",
            self.tree_len(),
            cx.processor(|pane, entries: Range<usize>, window, cx| {
                entries
                    .map(|entry| pane.draw_entry(entry, window, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.tree.scroll)
        .size_full();
        Some(
            div()
                .flex_none()
                .w_full()
                .h(ROW_HEIGHT * shown as f32)
                .border_b_1()
                .border_color(theme.border)
                .key_context(HISTORY_CONTEXT)
                .track_focus(&self.tree.focus)
                .on_action(cx.listener(Self::history_down))
                .on_action(cx.listener(Self::history_up))
                .on_action(cx.listener(Self::mark_change))
                .on_action(cx.listener(Self::show_change))
                .on_action(cx.listener(Self::focus_rows))
                .child(list),
        )
    }

    /// The bool is whether the entry can be selected: not a turn whose state was not kept or whose
    /// recording expired.
    fn entry_words(&self, index: usize, cx: &App) -> (String, bool) {
        let entries = self
            .tree
            .history
            .as_ref()
            .map_or(&[][..], |history| &history.entries[..]);
        let who = |session: &yaar_link::SessionId| {
            self.models
                .agents
                .read(cx)
                .session(session)
                .map_or("An agent".to_owned(), |session| session.title.clone())
        };
        match entries.get(index) {
            Some(HistoryEntry::Commit(commit)) => (
                format!(
                    "{} {}",
                    commit.id.get(..7).unwrap_or(&commit.id),
                    commit.summary
                ),
                true,
            ),
            Some(HistoryEntry::Turn { session, state, .. }) => match state {
                Some(_) => (format!("{}, a turn", who(session)), true),
                None => (
                    format!("{}, a turn (its state was not kept)", who(session)),
                    false,
                ),
            },
            Some(HistoryEntry::ExpiredTurn { session, .. }) => {
                (format!("{}, {EXPIRED_TURN}", who(session)), false)
            }
            None => {
                let count = self.tree.uncommitted.as_ref().map_or(0, Vec::len);
                (format!("Uncommitted changes ({count})"), true)
            }
        }
    }

    fn draw_entry(&self, index: usize, window: &Window, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let entries = self
            .tree
            .history
            .as_ref()
            .map_or(&[][..], |history| &history.entries[..]);
        let uncommitted = index >= entries.len();
        let selected = match self.tree.shown {
            ChangeShown::Entries { first, last } => !uncommitted && (first..=last).contains(&index),
            ChangeShown::Uncommitted => uncommitted,
            ChangeShown::Comparison | ChangeShown::Since { .. } => false,
        };
        let here = self.tree.cursor == Some(index) && self.tree.focus.is_focused(window);
        let marked = self.tree.marked.contains(&index);
        let (label, selectable) = self.entry_words(index, cx);
        let words_selector = if uncommitted {
            format!("review-history-uncommitted:{label}")
        } else {
            format!("review-history-{index}:{label}")
        };
        let row = div()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_l_2()
            .border_color(if here {
                theme.primary
            } else {
                transparent_black()
            })
            .when(selected, |row| row.bg(theme.secondary))
            .when(!selectable, |row| row.text_color(theme.muted_foreground))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |pane, event: &MouseDownEvent, window, cx| {
                    pane.click_entry(index, event.modifiers.shift, window, cx);
                }),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(10.))
                    .child(if marked { "●" } else { "" }),
            )
            .child(
                div()
                    .child(label)
                    .debug_selector(move || words_selector.clone()),
            );
        if uncommitted {
            row.debug_selector(|| "review-history-uncommitted".to_owned())
        } else {
            row.debug_selector(|| format!("review-history-{index}"))
        }
    }
}
