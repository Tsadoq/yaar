//! The tree of Changes: the history of the worktree above the files, which entry or range
//! of entries the files are the change of, and the keys and clicks that move between
//! them.

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

/// How many entries of the tree are in view at the most. A longer history scrolls inside
/// its own list, so that it never takes the files' room.
const ENTRIES_IN_VIEW: usize = 8;

/// Where the two versions of the files shown come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// The comparison of the workspace, read as slice 1 reads it.
    Comparison,
    /// Two versions the Keeper names: the old one, none where nothing came before, and
    /// the new one, none for the files on disk.
    Versions {
        old: Option<String>,
        new: Option<String>,
    },
}

/// Who is credited with each file of a list the Keeper made for a review: what
/// `CompareForReview` answered, kept beside the files it listed.
pub(crate) struct Credits {
    /// The review the list was asked for.
    pub(crate) review: ReviewId,
    /// The files with the sessions credited with them, as answered.
    pub(crate) files: Vec<CreditedFile>,
}

/// What the list of a change that starts at a version and ends at another was made for. It
/// says whether the list is a review's own, and so is made again when another review is
/// shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListedFor {
    /// No review: what changed since a round, a turn end of a session the pane holds no
    /// review of, an entry or a range of the tree, the files on disk. It stays as it is when
    /// another review is shown.
    NoReview,
    /// The review of a session, through `CompareForReview`: only the files it shows, each
    /// with its credits.
    Review(ReviewId),
    /// The review of no agent, through `CompareVersions`, as the review of a session that
    /// was shown before it left the change on screen. A session's review shown next lists
    /// the change again for itself.
    NoAgentReview,
}

impl ListedFor {
    /// The review `CompareForReview` is asked for, if it is.
    fn review(&self) -> Option<&ReviewId> {
        match self {
            Self::Review(review) => Some(review),
            Self::NoReview | Self::NoAgentReview => None,
        }
    }
}

/// What the pane holds of the tree of Changes.
pub(crate) struct Tree {
    /// The workspace the history and the uncommitted changes are of.
    pub(crate) of: Option<WorkspaceId>,
    /// The history, as the Keeper last answered.
    pub(crate) history: Option<History>,
    /// The last question for the history: of two answers, only the last asked counts.
    reading_history: Option<Task<()>>,
    /// The uncommitted changes of the review shown, as the Keeper last answered.
    pub(crate) uncommitted: Option<Vec<CreditedFile>>,
    /// The review the uncommitted changes were last asked for; none while they were not.
    uncommitted_for: Option<ReviewId>,
    reading_uncommitted: Option<Task<()>>,
    /// The entry under the cursor.
    pub(crate) cursor: Option<usize>,
    /// The entries marked for a range, in order.
    pub(crate) marked: Vec<usize>,
    /// The focus of the tree: its keys work while it has it.
    pub(crate) focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The credits of the files last listed through `CompareForReview`; none when they were
    /// listed by `CompareVersions`, which carries none (`shared.rs`).
    pub(crate) credits: Option<Credits>,
    /// The change whose files the pane shows.
    pub(crate) shown: ChangeShown,
    /// Whether `shown` is what the pane picked for a window that has no changes yet, and
    /// not what the reviewer selected: the comparison takes its place when it comes.
    pub(crate) first_picture: bool,
    /// Where the files of `shown` come from.
    pub(crate) wanted: Source,
    /// What the list of `wanted` was asked for. It is kept with `wanted` and not read off
    /// `credits`, which hold the review of the list last answered, not of the request still
    /// on its way.
    pub(crate) wanted_for: ListedFor,
    /// The question for the files of an entry or a range, while one is on its way.
    pub(crate) listing: Option<Task<()>>,
    /// What a turn end offered while the pane kept the selection; selecting anything
    /// takes it away.
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

/// The version an entry stands for: a commit's id, or the state recorded at a turn's end;
/// none for a turn whose state was not kept.
fn version_of(entry: &HistoryEntry) -> Option<&str> {
    match entry {
        HistoryEntry::Commit(commit) => Some(&commit.id),
        HistoryEntry::Turn { state, .. } => state.as_deref(),
        HistoryEntry::ExpiredTurn { .. } => None,
    }
}

/// The version a change that starts at entry `first` is counted from: the nearest version
/// above it, or the history's start.
fn before(history: &History, first: usize) -> Option<String> {
    history.entries[..first]
        .iter()
        .rev()
        .find_map(version_of)
        .or(history.start.as_deref())
        .map(str::to_owned)
}

/// The commit checked out: the newest commit of the history, or its start where it holds
/// none. The uncommitted changes are counted from it.
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
    /// How many entries the tree has: those of the history and the uncommitted changes.
    fn tree_len(&self) -> usize {
        self.tree
            .history
            .as_ref()
            .map_or(0, |history| history.entries.len() + 1)
    }

    /// Whether the tree is drawn: the history is here and there is something in it to
    /// select. A worktree with no commit and no turn and nothing uncommitted has no tree,
    /// and its changes are drawn as they always were.
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

    /// Whether the entry at `index` of the history can be selected.
    fn selectable(&self, index: usize) -> bool {
        self.tree
            .history
            .as_ref()
            .and_then(|history| history.entries.get(index))
            .and_then(version_of)
            .is_some()
    }

    /// Takes up the workspace on screen: when it is another than the one the tree is of,
    /// the tree is let go, and the history of the new one is asked for.
    pub(crate) fn follow_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.models.changes.read(cx).workspace().cloned();
        if workspace == self.tree.of {
            return;
        }
        // The keys must not be left without a place: the rows take the focus the tree had.
        if self.tree.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
        self.tree = Tree::new(cx);
        self.tree.of = workspace;
        self.listed = None;
        self.read_history(cx);
        cx.notify();
    }

    /// Asks the Keeper for the history of the workspace on screen. Only the answer to the
    /// last question is taken.
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

    /// Shows an answer for the history, and keeps what is selected in it when what was
    /// selected is still there.
    fn history_answered(&mut self, history: History, cx: &mut Context<Self>) {
        // An answer that tells nothing new, as the second of two questions asked close
        // together, leaves what is shown as it is.
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
                // What was selected stays when the entries are still what they were; the
                // files are asked for again only when the version they lead from or to
                // has changed.
                if !self.select_entries(first, last, !self.tree.first_picture, cx) {
                    self.back_to_comparison(cx);
                }
            }
            ChangeShown::Uncommitted => {
                // The files are read again only when the commit they are counted from
                // is another.
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
                // A window that has no changes yet shows the last turn that was recorded:
                // the history is read from the Keeper's records, and the changes need git.
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
        // Flow reads the newest turn of the history, and what a turn's change counts from
        // where the turn says nothing.
        self.flow_history(cx);
        self.live_history(cx);
        cx.notify();
    }

    /// Asks the Keeper for the uncommitted changes of the review shown. Nothing is asked
    /// while the pane holds no review: they are asked for when it does.
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

    /// The review shown may have changed: the uncommitted changes are those of the review
    /// shown, so they are asked for again when it is another than the last asked for.
    pub(crate) fn review_shown(&mut self, cx: &mut Context<Self>) {
        let now = self.review().map(|review| &review.id);
        if now.is_some() && self.tree.uncommitted_for.as_ref() != now {
            self.read_uncommitted(cx);
        }
        self.list_again_for_review_shown(cx);
    }

    /// A change listed for a review is that review's own list, the review of no agent's
    /// included. When another review is shown, the change shown is listed again for it:
    /// through `CompareForReview` for a session's review, through `CompareVersions` for the
    /// review of no agent. A change listed with no review, as what changed since a round is,
    /// stays as it is.
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
        // The files on disk are read again with the list: that is what a change of the
        // files is told by.
        if self.tree.shown == ChangeShown::Uncommitted {
            self.select_uncommitted(cx);
        }
        cx.notify();
    }

    /// Selects the entries `first` to `last` of the history, both included, as the change
    /// to show: from the version before `first` to the version of `last`. `by_reviewer`
    /// says whether the reviewer chose them, and not the pane for want of changes. False when no such change can be shown: the
    /// history is not here, an entry does not exist, or an end of the range holds no
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

    /// Selects what changed between two versions, `from` to `to`, as the change to show:
    /// what changed since a round was sent, listed with no credits.
    pub(crate) fn select_since(
        &mut self,
        from: Option<String>,
        to: String,
        cx: &mut Context<Self>,
    ) {
        self.select_since_for(from, to, ListedFor::NoReview, cx);
    }

    /// Selects what changed between two versions as `select_since` does, listed for
    /// `listed_for`: for the review of an agent session, only the files it shows, each with
    /// its credits; for the review of no agent, which has none, every file with none.
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

    /// Lists the files of the change from `old` to `new`, none for the files on disk, and
    /// shows them, unless they are shown already. For a review of a session the list is the
    /// one `CompareForReview` makes for it, and its credits are kept beside the files.
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
        // The same two versions listed for another review are another list, also while the
        // request for the one before is still on its way. A list for a review that the
        // Keeper refused is asked for again.
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
            // The Keeper compares two versions: with none before the change, there is
            // nothing to list its files from.
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
        // A selection that takes the place of this one drops the task, and its answer
        // with it.
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

    /// Selects the uncommitted changes of the review shown as the change to show: from the
    /// commit checked out to the files on disk, the files those of the list.
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

    /// Puts the comparison of the workspace back in the place of an entry or a range.
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

    /// What [`FocusHistory`] does: puts the focus on the tree, with the cursor on the
    /// first entry the first time.
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

    /// What [`FocusRows`] does: gives the focus back to the rows of the diff.
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

    /// Moves the cursor by `by` entries, and stays at the first and at the last.
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

    /// What [`HistoryDown`] does.
    pub(crate) fn history_down(&mut self, _: &HistoryDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step_cursor(1, cx);
    }

    /// What [`HistoryUp`] does.
    pub(crate) fn history_up(&mut self, _: &HistoryUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step_cursor(-1, cx);
    }

    /// What [`MarkChange`] does: marks the entry under the cursor for the range, or takes
    /// its mark away. An entry that cannot be selected, and the uncommitted changes, which
    /// no range holds, are not marked.
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

    /// What [`ShowChange`] does: shows the change of the marked entries, from the first
    /// to the last, or of the entry under the cursor when none is marked.
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

    /// Selects the entry at `index` of the tree, the uncommitted changes being the one
    /// after the last of the history: alone, or, with `extend`, as the range from the
    /// entries shown to it.
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

    /// What a click on the entry at `index` does: gives the tree the focus, puts the cursor
    /// there and shows its change at once, or with Shift the range from the one shown.
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
            // A turn with no state to show.
            return cx.notify();
        }
        self.select(index, shift, cx);
    }

    /// The tree, drawn above the files; none when there is nothing in it to select.
    pub(crate) fn tree_view(&self, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        if !self.has_tree() {
            if self.tree.focus.is_focused(window) {
                // The tree is gone, and the keys are left without a place: the rows take
                // the focus once this frame is drawn.
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

    /// What the row of entry `index` says, and whether the entry can be selected. A turn
    /// whose state was not kept, and one whose recording expired, can not.
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

    /// The entry at `index` of the tree as a row.
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
        // The words the row draws are in the selector of the text, as the other labels'
        // are: "review-history-<index>:<words>", "review-history-uncommitted:<words>".
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
