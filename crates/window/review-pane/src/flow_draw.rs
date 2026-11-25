//! Drawing Flow, and the two-row header at the top of the review pane. The header lies outside
//! the element whose listeners pause follow, and the view/form/follow controls call `steered`,
//! never `acted`.

use std::ops::{Range, RangeInclusive};

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yaar_frame::SetBase;
use yaar_link::{Comparison, FlowForm, SessionState, Step, TurnEnd};
use yaar_ui::diff::DiffStats;
use yaar_ui::metrics::{SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, tabular};
use yaar_ui::pieces::{
    Button, ButtonKind, Chip, CountBadge, EmptyState, Segmented, State, StateDot, Tabs, Toggle,
    Tone,
};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::draw::{InView, comparison_sentence, short};
use crate::{
    AttachReview, ChangeShown, Commit, DetachReview, FileContent, Following, Layout, MarkReviewed,
    ReviewPane, TimelineEntry, ToggleFollow, ToggleLayout, View,
};

const TIMELINE_ROW_HEIGHT: Pixels = px(30.);

impl ReviewPane {
    pub(crate) fn render_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = palette(cx);
        let in_flow = self.view == View::Flow;

        let workspace_info = self
            .models
            .changes
            .read(cx)
            .workspace()
            .and_then(|id| self.models.workspaces.read(cx).workspace(id))
            .map(|workspace| {
                let name = workspace
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| workspace.path.display().to_string());
                (name, workspace.head.clone())
            });

        let comparison_chip =
            if let (Some(changes), ChangeShown::Comparison) = (&self.changes, &self.tree.shown) {
                let offers_base = matches!(changes.comparison, Comparison::NothingToCompare { .. });
                let label = match (&workspace_info, &changes.comparison) {
                    (
                        Some((_, Some(yaar_link::Head::Branch(branch)))),
                        Comparison::SinceBase { base },
                    ) => {
                        format!("{branch} ← {}", base.name)
                    }
                    _ => comparison_sentence(&changes.comparison),
                };
                Some((label, offers_base))
            } else {
                None
            };

        let reviewed_chip = self
            .review()
            .and_then(|review| review.reviewed.as_deref())
            .map(|state| {
                Chip::new(format!("Reviewed at {}", short(state)))
                    .tone(Tone::Ok)
                    .selector("review-reviewed")
            });

        let sessions = self.session_chips(cx);
        let sessions_group = (!sessions.is_empty()).then(|| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    div()
                        .typed(TypeRole::Small)
                        .text_color(palette.text_dim)
                        .whitespace_nowrap()
                        .child("Reviewing:"),
                )
                .children(sessions)
        });
        let server_chip = self.render_server_chip(cx);

        let (total_added, total_removed) = self.total_diff_stats();
        let file_stats = (!self.files.is_empty()).then(|| {
            let count = self.files.len();
            let word = if count == 1 { "file" } else { "files" };
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .typed(TypeRole::Small)
                        .text_color(palette.text_muted)
                        .whitespace_nowrap()
                        .child(format!("{count} {word}")),
                )
                .child(DiffStats::new(total_added, total_removed).small())
        });

        let unsent = self.unsent_comment_count();
        let attached = self
            .review()
            .and_then(|review| review.attached.as_ref())
            .is_some();
        let has_workspace = self.models.changes.read(cx).workspace().is_some();
        let has_review = self.review().is_some();

        let top_row = has_workspace.then(|| {
            let mut row = div()
                .flex_none()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(SPACE_3)
                .px(SPACE_5)
                .py(SPACE_3)
                .border_b_1()
                .border_color(palette.separator)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                        pane.focus.focus(window, cx);
                    }),
                );

            if let Some((name, _)) = &workspace_info {
                row = row.child(
                    div()
                        .flex_none()
                        .typed(TypeRole::Strong)
                        .text_color(palette.text)
                        .child(name.clone()),
                );
            }

            if let Some((label, offers_base)) = comparison_chip {
                if offers_base {
                    row = row.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .typed(TypeRole::Small)
                            .text_color(palette.text_muted)
                            .child(label),
                    );
                    row = row.child(
                        Button::new("review-offer-base", "Set a base…")
                            .kind(ButtonKind::Primary)
                            .small()
                            .selector("review-offer-base")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(SetBase), cx)
                            }),
                    );
                } else {
                    row = row.child(
                        Chip::new(label)
                            .mono()
                            .max_width(px(210.))
                            .on_click("review-base-chip", |_, window, cx| {
                                window.dispatch_action(Box::new(SetBase), cx)
                            }),
                    );
                }
            }

            let mut attach = Button::new(
                "review-attach-review",
                if attached {
                    "Attached"
                } else {
                    "Attach review"
                },
            )
            .kind(ButtonKind::Primary)
            .small()
            .disabled(!has_review)
            .on_click(cx.listener(move |pane, _: &ClickEvent, window, cx| {
                if attached {
                    pane.detach_review(&DetachReview, window, cx);
                } else {
                    pane.attach_review(&AttachReview, window, cx);
                }
            }));
            if unsent > 0 {
                attach = attach.count(unsent);
            }

            let actions = div()
                .flex_none()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    Button::new("review-mark-reviewed", "Mark reviewed")
                        .small()
                        .disabled(!has_review)
                        .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                            pane.mark_reviewed(&MarkReviewed, window, cx);
                        })),
                )
                .child(attach)
                .child(
                    Button::new("review-commit", "Commit…")
                        .small()
                        .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                            pane.commit(&Commit, window, cx);
                        })),
                );

            row = row
                .children(reviewed_chip)
                .children(sessions_group)
                .children(server_chip)
                .children(file_stats)
                .child(div().flex_1())
                .child(actions);
            row
        });

        let working = self
            .sessions
            .iter()
            .any(|session| session.state == SessionState::Works);
        let file_count = (!self.files.is_empty()).then_some(self.files.len());
        let entity = cx.entity();
        let tabs = Tabs::new("review-views")
            .selector("review-view")
            .tab("changes", "Changes", file_count, None)
            .tab("flow", "Flow", None, working.then_some(State::Working))
            .selected(if in_flow { 1 } else { 0 })
            .on_select({
                let entity = entity.clone();
                move |index, _, cx| {
                    entity.update(cx, |pane, cx| {
                        pane.steered(cx);
                        let view = if index == 0 {
                            View::Changes
                        } else {
                            View::Flow
                        };
                        pane.show_view(view, cx);
                    });
                }
            });

        let follow_label = match self.following {
            Following::On => "Follow",
            Following::Paused { .. } => "Follow: paused",
            Following::Off => "Follow",
        };
        let follow = Toggle::new("flow-follow", follow_label)
            .on(self.following == Following::On)
            .paused(matches!(self.following, Following::Paused { .. }))
            .selector("flow-follow")
            .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                pane.toggle_follow(&ToggleFollow, window, cx);
            }));

        let form_switch = if in_flow {
            Segmented::new("flow-form")
                .selector("flow-form")
                .segment("timeline", "Timeline")
                .segment("live", "Live diff")
                .selected(if self.form == FlowForm::Timeline {
                    0
                } else {
                    1
                })
                .on_select(move |index, _, cx| {
                    entity.update(cx, |pane, cx| {
                        pane.steered(cx);
                        let form = if index == 0 {
                            FlowForm::Timeline
                        } else {
                            FlowForm::LiveDiff
                        };
                        pane.choose_form(form, cx);
                    });
                })
        } else {
            Segmented::new("review-layout")
                .selector("review-layout")
                .segment("unified", "Unified")
                .segment("split", "Split")
                .selected(if self.layout == Layout::Unified { 0 } else { 1 })
                .on_select(move |index, window, cx| {
                    entity.update(cx, |pane, cx| {
                        let wanted = if index == 0 {
                            Layout::Unified
                        } else {
                            Layout::Split
                        };
                        if pane.layout != wanted {
                            pane.toggle_layout(&ToggleLayout, window, cx);
                        }
                    });
                })
        };

        let tools_row = div()
            .debug_selector(|| "review-bar".to_owned())
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_5)
            .py(SPACE_3)
            .overflow_hidden()
            .whitespace_nowrap()
            .border_b_1()
            .border_color(palette.separator)
            // The focus goes to the rows, where the keys of the pane work.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                    pane.focus.focus(window, cx);
                }),
            )
            .child(tabs)
            .child(div().flex_1())
            .child(follow)
            .child(form_switch);

        Some(
            div()
                .flex_none()
                .w_full()
                .flex()
                .flex_col()
                .children(top_row)
                .child(tools_row)
                .into_any_element(),
        )
    }

    fn total_diff_stats(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;
        for held in &self.files {
            if let FileContent::Diff(shown) = &held.content {
                for hunk in shown.diff.hunks() {
                    added += hunk.new.len();
                    removed += hunk.old.len();
                }
            }
        }
        (added, removed)
    }

    fn unsent_comment_count(&self) -> usize {
        self.review().map_or(0, |review| {
            review
                .threads
                .iter()
                .filter(|thread| !thread.resolved)
                .flat_map(|thread| &thread.comments)
                .filter(|comment| comment.round.is_none())
                .count()
        })
    }

    pub(crate) fn render_flow(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let paused = self.follow_paused_pill(cx);
        let offer = self.follow_offer_card(cx);
        let list = match self.form {
            FlowForm::Timeline => self.render_timeline(cx),
            FlowForm::LiveDiff => div()
                .debug_selector(|| "flow-live".to_owned())
                .size_full()
                .child(self.render_live(cx))
                .into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(list))
            .children(offer)
            .children(paused)
            .into_any_element()
    }

    fn render_timeline(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.flow.entries().is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    EmptyState::new("No turn of the agent to show yet")
                        .line(
                            "Flow shows every edit, file read, and command while the agent works.",
                        )
                        .selector("flow-empty"),
                )
                .into_any_element();
        }
        uniform_list(
            "flow-timeline",
            self.flow.entries().len(),
            cx.processor(|pane, rows: Range<usize>, _, cx| {
                let cursor = pane.flow.cursor();
                let at = pane.at.as_ref().and_then(|at| pane.flow.row_of(at));
                rows.map(|index| {
                    pane.draw_timeline_entry(index, cursor == Some(index), at == Some(index), cx)
                })
                .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.flow.scroll)
        .with_decoration(InView(self.flow.in_view.clone()))
        .size_full()
        .into_any_element()
    }

    fn draw_timeline_entry(
        &self,
        index: usize,
        reviewer: bool,
        at: bool,
        cx: &Context<Self>,
    ) -> Div {
        let palette = palette(cx);
        let entry = &self.flow.entries()[index];
        let words = entry_words(entry);
        let base = div()
            .debug_selector(|| format!("flow-entry-{index}"))
            .h(TIMELINE_ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .px(SPACE_6)
            .overflow_hidden()
            .whitespace_nowrap()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                    pane.click_timeline_entry(index, cx)
                }),
            );

        match entry {
            TimelineEntry::Turn { turn, .. } => {
                let step_count = self.flow.step_count(turn);
                let running = self.flow.is_running(turn);
                let dot = StateDot::new(if running { State::Working } else { State::Idle });
                base.bg(palette.window)
                    .border_t_1()
                    .border_b_1()
                    .border_color(palette.separator)
                    .child(dot)
                    .child(
                        // The words are in the selector: a test reads what was handed to the text
                        // system.
                        div()
                            .debug_selector({
                                let words = words.clone();
                                move || format!("flow-entry-text-{index}:{words}")
                            })
                            .min_w_0()
                            .truncate()
                            .typed(TypeRole::Label)
                            .text_color(palette.text)
                            .child(words.to_uppercase()),
                    )
                    .child(div().flex_1())
                    .when(step_count > 0, |row| row.child(CountBadge::new(step_count)))
                    .when(step_count == 0 && running, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .typed(TypeRole::Tiny)
                                .text_color(palette.text_dim)
                                .child("Working…"),
                        )
                    })
            }
            TimelineEntry::Step { number, step, .. } => {
                let (badge_label, badge_tone, detail, extra) = match step {
                    Step::Read { path } => ("READ", Tone::Plain, path.display().to_string(), None),
                    Step::Edit { path, lines } => {
                        let spans = format_line_ranges(lines.as_deref());
                        let extra = (!spans.is_empty()).then(|| format!("lines {spans}"));
                        ("EDIT", Tone::Ok, path.display().to_string(), extra)
                    }
                    Step::Write { path } => ("WRITE", Tone::Ok, path.display().to_string(), None),
                    Step::Command { command } => (
                        "RUN",
                        Tone::Info,
                        command.lines().next().unwrap_or_default().to_owned(),
                        None,
                    ),
                };
                let text_colour = if at { palette.link } else { palette.text };
                base.border_l_2()
                    .border_color(if at {
                        palette.focus
                    } else {
                        transparent_black()
                    })
                    .when(reviewer, |row| row.bg(palette.raised))
                    .child(
                        div()
                            .flex_none()
                            .w(px(22.))
                            .flex()
                            .justify_end()
                            .typed(TypeRole::Tiny)
                            .font_features(tabular())
                            .text_color(palette.text_dim)
                            .child(number.to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(62.))
                            .flex()
                            .items_center()
                            .child(Chip::new(badge_label).tone(badge_tone)),
                    )
                    .child(
                        // The words are in the selector: a test reads what was handed to the text
                        // system.
                        div()
                            .debug_selector({
                                let words = words.clone();
                                move || format!("flow-entry-text-{index}:{words}")
                            })
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(SPACE_3)
                            .overflow_hidden()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .typed(TypeRole::CodeSmall)
                                    .text_color(text_colour)
                                    .child(detail),
                            )
                            .children(extra.map(|spans| {
                                div()
                                    .flex_none()
                                    .typed(TypeRole::Small)
                                    .text_color(palette.text_dim)
                                    .whitespace_nowrap()
                                    .child(spans)
                            })),
                    )
            }
            TimelineEntry::End { how, .. } => {
                let (colour, state) = match how {
                    Some(TurnEnd::Finished) => (palette.ok, State::Ok),
                    Some(TurnEnd::Interrupted) => (palette.warn, State::NeedsYou),
                    None => (palette.text_muted, State::Idle),
                };
                base.border_l_2()
                    .border_color(transparent_black())
                    .when(reviewer, |row| row.bg(palette.raised))
                    .child(StateDot::new(state))
                    .child(
                        div()
                            .debug_selector({
                                let words = words.clone();
                                move || format!("flow-entry-text-{index}:{words}")
                            })
                            .min_w_0()
                            .truncate()
                            .typed(TypeRole::Meta)
                            .text_color(colour)
                            .child(words),
                    )
            }
        }
    }
}

pub(crate) fn format_line_ranges(lines: Option<&[RangeInclusive<u32>]>) -> String {
    lines
        .into_iter()
        .flatten()
        .map(|range| {
            if range.start() == range.end() {
                range.start().to_string()
            } else {
                format!("{}-{}", range.start(), range.end())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn entry_words(entry: &TimelineEntry) -> String {
    match entry {
        TimelineEntry::Turn { title, .. } => title.clone(),
        TimelineEntry::Step { step, .. } => match step {
            Step::Read { path } => format!("Read {}", path.display()),
            Step::Edit { path, lines } => {
                let lines = format_line_ranges(lines.as_deref());
                match lines.is_empty() {
                    true => format!("Edit {}", path.display()),
                    false => format!("Edit {} lines {lines}", path.display()),
                }
            }
            Step::Write { path } => format!("Write {}", path.display()),
            Step::Command { command } => {
                format!("Run {}", command.lines().next().unwrap_or_default())
            }
        },
        TimelineEntry::End { how, .. } => match how {
            Some(TurnEnd::Finished) => "Turn finished".to_owned(),
            Some(TurnEnd::Interrupted) => "Turn interrupted".to_owned(),
            None => "Turn ended".to_owned(),
        },
    }
}
