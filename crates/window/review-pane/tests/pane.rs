mod common;

use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Bounds, ClipboardItem, Entity, Modifiers, Pixels, ScrollDelta, ScrollEvent, TestAppContext,
    VisualTestContext, point, px, size,
};
use yaar_diff::{Row, Side};
use yaar_frame::{Place, SetBase};
use yaar_link::{
    Anchor, Author, Changes, Comment, CommentId, Comparison, Content, Fact, FileSides, FileStatus,
    FlowForm, Lines, NoComparison, Refusal, Reply, Request, RequestKind, Setting, Side as LinkSide,
    Thread, ThreadId, ThreadState, TurnEndSelects,
};
use yaar_review_pane::{
    FILES_ON_THEIR_WAY, Following, HISTORY_CONTEXT, Layout, PaneRow, Position, REFERENCES_CONTEXT,
    ReviewPane, Unshown, View, controls,
};

use common::{
    Stage, WINDOW, bounds_of_row, commit, default_base, drawn, file, id, link_up, open,
    open_staged, open_window, read_file, repository, reviews_of, rows, since_main, texts,
    wait_for_files, wait_until, workspace,
};

fn reading(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<String> {
    pane.read_with(cx, |pane, _| {
        let line = |path: &PathBuf, sign: char, side: Side, line: u32| {
            let position = Position {
                path: path.clone(),
                side,
                line,
            };
            let text = pane
                .text(&position)
                .unwrap_or_else(|| panic!("the pane shows {position:?} and has no text for it"));
            format!("{sign}{text}")
        };
        let mut read = Vec::new();
        for row in pane.rows() {
            match row {
                PaneRow::File { path, .. } => read.push(format!("# {}", path.display())),
                PaneRow::Diff { path, row } => match row {
                    Row::Context { old, new } => {
                        assert_eq!(
                            line(path, ' ', Side::Old, *old),
                            line(path, ' ', Side::New, *new)
                        );
                        read.push(line(path, ' ', Side::New, *new));
                    }
                    Row::Removed { old } => read.push(line(path, '-', Side::Old, *old)),
                    Row::Added { new } => read.push(line(path, '+', Side::New, *new)),
                    Row::Changed { old, new } => {
                        read.extend(old.map(|old| line(path, '-', Side::Old, old)));
                        read.extend(new.map(|new| line(path, '+', Side::New, new)));
                    }
                    Row::Skipped { old, .. } => read.push(format!("… {}", old.len())),
                },
                PaneRow::Commit { .. }
                | PaneRow::Unshown { .. }
                | PaneRow::Thread { .. }
                | PaneRow::Comment { .. }
                | PaneRow::ThreadsElsewhere
                | PaneRow::Read { .. } => {}
            }
        }
        read
    })
}

fn assert_lines_drawn_as_read(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) {
    let (layout, shown, in_view) = pane.read_with(cx, |pane, _| {
        (pane.layout(), pane.rows().to_vec(), pane.rows_in_view())
    });
    // A line of text is a pixel higher than its row and is cut to it, so its middle is what must
    // lie in the row.
    let holds = |outer: Bounds<Pixels>, inner: Bounds<Pixels>| {
        outer.contains(&inner.center())
            && outer.left() <= inner.left()
            && inner.right() <= outer.right()
    };
    let mut lines = 0;
    for index in in_view {
        let PaneRow::Diff { path, row } = &shown[index] else {
            continue;
        };
        let whole = bounds_of_row(index, cx);
        if layout == Layout::Split {
            let half = |name: &str, cx: &mut VisualTestContext| {
                drawn(&format!("review-row-{index}-{name}"), cx)
                    .unwrap_or_else(|| panic!("row {index} has no {name} half"))
            };
            let (old, new) = (half("old", cx), half("new", cx));
            assert!(
                old.right() <= new.left(),
                "row {index}: the old half {old:?} is not left of the new half {new:?}"
            );
        }
        for (side, name) in [(Side::Old, "old"), (Side::New, "new")] {
            let once = layout == Layout::Unified && matches!(row, Row::Context { .. });
            let Some(line) = row.line(side).filter(|_| !(once && side == Side::Old)) else {
                continue;
            };
            let position = Position {
                path: path.clone(),
                side,
                line,
            };
            let text = pane
                .read_with(cx, |pane, _| pane.text(&position))
                .unwrap_or_else(|| panic!("the pane shows {position:?} and has no text for it"));
            let place = drawn(&format!("review-text-{index}-{name}:{text}"), cx)
                .unwrap_or_else(|| panic!("row {index} does not draw {text:?} for {position:?}"));
            assert!(place.size.width > px(0.) && place.size.height > px(0.));
            assert!(
                holds(whole, place),
                "{position:?} is drawn at {place:?}, outside row {index} at {whole:?}"
            );
            if layout == Layout::Split {
                let half = drawn(&format!("review-row-{index}-{name}"), cx)
                    .unwrap_or_else(|| panic!("row {index} has no {name} half"));
                assert!(
                    holds(half, place),
                    "{position:?} is drawn in the other half"
                );
            }
            lines += 1;
        }
    }
    assert!(lines > 0, "no line of a diff is in view");
}

#[gpui_kit::test]
fn the_pane_shows_the_changes_of_the_workspace_line_by_line(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let numbers = PathBuf::from("src/numbers.txt");
    let notes = PathBuf::from("src/notes.txt");
    let changes = since_main(
        vec![
            commit('c', "Count to five"),
            commit('b', "Write two as a digit"),
        ],
        vec![
            file("src/numbers.txt", FileStatus::Modified),
            file("src/notes.txt", FileStatus::Added),
        ],
    );
    assert_eq!(rows(&stage.pane, &mut cx), []);

    stage.keeper.show(
        &changes,
        |file| match file.path.to_str() {
            Some("src/numbers.txt") => texts(
                "one\ntwo\nthree\nfour\n",
                "zero\none\n2\nthree\nfour\nfive\n",
            ),
            _ => Ok(FileSides {
                old: Content::Absent,
                new: Content::Text(b"alpha\nbeta\n".to_vec()),
            }),
        },
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);

    let diff = |path: &PathBuf, row: Row| PaneRow::Diff {
        path: path.clone(),
        row,
    };
    let shown = rows(&stage.pane, &mut cx);
    assert_eq!(
        shown[2..],
        [
            PaneRow::File {
                path: numbers.clone(),
                status: FileStatus::Modified
            },
            diff(&numbers, Row::Added { new: 0 }),
            diff(&numbers, Row::Context { old: 0, new: 1 }),
            diff(&numbers, Row::Removed { old: 1 }),
            diff(&numbers, Row::Added { new: 2 }),
            diff(&numbers, Row::Context { old: 2, new: 3 }),
            diff(&numbers, Row::Context { old: 3, new: 4 }),
            diff(&numbers, Row::Added { new: 5 }),
            PaneRow::File {
                path: notes.clone(),
                status: FileStatus::Added
            },
            diff(&notes, Row::Added { new: 0 }),
            diff(&notes, Row::Added { new: 1 }),
        ]
    );
    assert_eq!(
        reading(&stage.pane, &mut cx),
        [
            "# src/numbers.txt",
            "+zero",
            " one",
            "-two",
            "+2",
            " three",
            " four",
            "+five",
            "# src/notes.txt",
            "+alpha",
            "+beta",
        ]
    );
    stage.pane.read_with(&cx, |pane, _| {
        let at = |path: &PathBuf, side, line| {
            pane.text(&Position {
                path: path.clone(),
                side,
                line,
            })
        };
        assert_eq!(at(&numbers, Side::Old, 3).as_deref(), Some("four"));
        assert_eq!(at(&numbers, Side::Old, 2).as_deref(), Some("three"));
        assert_eq!(at(&numbers, Side::New, 3).as_deref(), Some("three"));
        assert_eq!(at(&numbers, Side::New, 2).as_deref(), Some("2"));
        assert_eq!(at(&numbers, Side::Old, 4), None);
        assert_eq!(at(&notes, Side::Old, 0), None);
        assert_eq!(at(&PathBuf::from("src/other.txt"), Side::New, 0), None);
    });

    let in_view = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view());
    assert_eq!(in_view, 0..shown.len());
    let tops: Vec<_> = in_view
        .map(|index| bounds_of_row(index, &mut cx).top())
        .collect();
    assert!(tops.is_sorted_by(|above, below| above < below), "{tops:?}");
    assert_lines_drawn_as_read(&stage.pane, &mut cx);

    let first_line = bounds_of_row(3, &mut cx).center();
    cx.simulate_click(first_line, Modifiers::none());
    cx.simulate_keystrokes("s");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.layout()),
        Layout::Split
    );
    let split = rows(&stage.pane, &mut cx);
    assert_eq!(
        split[2..8],
        [
            PaneRow::File {
                path: numbers.clone(),
                status: FileStatus::Modified
            },
            diff(
                &numbers,
                Row::Changed {
                    old: None,
                    new: Some(0)
                }
            ),
            diff(&numbers, Row::Context { old: 0, new: 1 }),
            diff(
                &numbers,
                Row::Changed {
                    old: Some(1),
                    new: Some(2)
                }
            ),
            diff(&numbers, Row::Context { old: 2, new: 3 }),
            diff(&numbers, Row::Context { old: 3, new: 4 }),
        ]
    );
    assert_lines_drawn_as_read(&stage.pane, &mut cx);
}

fn change(stage: &mut Stage, changes: &Changes, cx: &mut VisualTestContext) {
    stage.keeper.tell(Fact::ChangesChanged {
        workspace: changes.workspace.clone(),
    });
    stage
        .keeper
        .show(changes, |_| texts("one\n", "one\ntwo\n"), cx);
    let comparison = changes.comparison.clone();
    wait_until(cx, "the rows of the new changes", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.comparison()
                .is_some_and(|(shown, _)| *shown == comparison)
        })
    });
    wait_for_files(&stage.pane, cx);
}

#[gpui_kit::test]
fn above_the_changes_the_pane_shows_the_comparison_and_its_commits_and_offers_a_base_where_there_is_none(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "main")], cx);
    let asked_for_a_base = Rc::new(Cell::new(0));
    cx.update(|_, cx| {
        let asked = asked_for_a_base.clone();
        cx.on_action(move |_: &SetBase, _| asked.set(asked.get() + 1));
    });

    let files = vec![file("notes.txt", FileStatus::Modified)];
    let heading = PaneRow::File {
        path: PathBuf::from("notes.txt"),
        status: FileStatus::Modified,
    };
    let with = |comparison: Comparison, commits: Vec<yaar_link::Commit>| Changes {
        workspace: id('1'),
        comparison,
        counted_from: Some(common::COMMIT.to_owned()),
        commits,
        files: files.clone(),
    };
    let commit_row = |commit: &yaar_link::Commit| PaneRow::Commit {
        id: commit.id.clone(),
        summary: commit.summary.clone(),
    };

    let compared = [
        with(
            Comparison::SinceBase {
                base: default_base("main"),
            },
            vec![
                commit('c', "Third"),
                commit('b', "Second"),
                commit('a', "First"),
            ],
        ),
        with(
            Comparison::NotOnUpstream {
                base: default_base("origin/main"),
                upstream: "origin/main".to_owned(),
            },
            vec![
                commit('e', "Not pushed yet"),
                commit('d', "Neither is this"),
            ],
        ),
        with(
            Comparison::OnNoRemoteBranch {
                base: default_base("main"),
            },
            vec![commit('f', "On no branch of the remote")],
        ),
    ];
    for (nth, changes) in compared.iter().enumerate() {
        if nth == 0 {
            stage
                .keeper
                .show(changes, |_| texts("one\n", "one\ntwo\n"), &mut cx);
            wait_for_files(&stage.pane, &mut cx);
        } else {
            change(&mut stage, changes, &mut cx);
        }
        let shown = rows(&stage.pane, &mut cx);
        let commits = changes.commits.len();
        assert_eq!(
            stage.pane.read_with(&cx, |pane, _| pane
                .comparison()
                .map(|(c, o)| (c.clone(), o))),
            Some((changes.comparison.clone(), false))
        );
        assert_eq!(
            shown[..commits],
            changes.commits.iter().map(commit_row).collect::<Vec<_>>()[..]
        );
        assert_eq!(shown[commits], heading);
        assert_eq!(
            drawn("review-offer-base", &mut cx),
            None,
            "{:?}",
            changes.comparison
        );
    }

    let bases = [
        (NoComparison::NoRemote, Some(default_base("main"))),
        (NoComparison::NoBase, None),
        (NoComparison::BaseGone, Some(default_base("gone"))),
        (NoComparison::NoSharedCommit, Some(default_base("main"))),
    ];
    for (clicks, (why, base)) in bases.into_iter().enumerate() {
        let changes = with(Comparison::NothingToCompare { base, why }, Vec::new());
        change(&mut stage, &changes, &mut cx);
        let shown = rows(&stage.pane, &mut cx);
        assert_eq!(
            stage.pane.read_with(&cx, |pane, _| pane
                .comparison()
                .map(|(c, o)| (c.clone(), o))),
            Some((changes.comparison.clone(), true))
        );
        assert_eq!(shown[0], heading, "{why:?}");
        assert_eq!(
            shown[1..],
            [
                PaneRow::Diff {
                    path: PathBuf::from("notes.txt"),
                    row: Row::Context { old: 0, new: 0 }
                },
                PaneRow::Diff {
                    path: PathBuf::from("notes.txt"),
                    row: Row::Added { new: 1 }
                },
            ]
        );

        let offer = drawn("review-offer-base", &mut cx)
            .unwrap_or_else(|| panic!("no base is offered for {why:?}"));
        assert!(offer.bottom() <= bounds_of_row(0, &mut cx).top());
        assert_eq!(asked_for_a_base.get(), clicks);
        cx.simulate_click(offer.center(), Modifiers::none());
        assert_eq!(asked_for_a_base.get(), clicks + 1, "{why:?}");
    }
}

/// A diff longer than the window, in two changes: forty lines replaced, then, two unchanged
/// lines further down, one line replaced. The first change takes 80 rows unified and 40 split,
/// so every line below it sits forty rows higher after a switch to the split view.
fn two_changes() -> (String, String) {
    let block = |word: &str| {
        (0..40)
            .map(|line| format!("{word} block line {line}\n"))
            .collect::<String>()
    };
    let tail: String = (0..10).map(|line| format!("tail {line}\n")).collect();
    (
        format!("{}keep a\nkeep b\nremove me\nkeep c\n{tail}", block("old")),
        format!("{}keep a\nkeep b\nadded here\nkeep c\n{tail}", block("new")),
    )
}

fn open_on_one_file(
    old: &str,
    new: &str,
    cx: &mut TestAppContext,
) -> (Stage, PathBuf, VisualTestContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(Vec::new(), vec![file("src/long.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(old, new), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let first_line = bounds_of_row(1, &mut cx).center();
    cx.simulate_click(first_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    (stage, PathBuf::from("src/long.txt"), cx)
}

fn position(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Position {
    pane.read_with(cx, |pane, _| pane.position().cloned())
        .expect("the reviewer is on a line")
}

fn walk_down_to(wanted: &Position, pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) {
    for _ in 0..500 {
        if position(pane, cx) == *wanted {
            return;
        }
        cx.simulate_keystrokes("j");
    }
    panic!("j never came to {wanted:?}");
}

#[gpui_kit::test]
fn switching_between_the_unified_and_the_split_view_keeps_the_line_the_reviewer_was_on(
    cx: &mut TestAppContext,
) {
    let (old, new) = two_changes();
    let (stage, path, mut cx) = open_on_one_file(&old, &new, cx);
    let pane = &stage.pane;
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane.layout()),
        Layout::Unified
    );

    let at = |side, line| Position {
        path: path.clone(),
        side,
        line,
    };
    let replaced = Row::Changed {
        old: Some(42),
        new: Some(42),
    };
    let unchanged = Row::Context { old: 43, new: 43 };
    let lines = [
        (
            at(Side::Old, 42),
            "remove me",
            Row::Removed { old: 42 },
            replaced.clone(),
        ),
        (
            at(Side::New, 42),
            "added here",
            Row::Added { new: 42 },
            replaced,
        ),
        (at(Side::New, 43), "keep c", unchanged.clone(), unchanged),
    ];

    for (line, text, unified, split) in lines {
        walk_down_to(&line, pane, &mut cx);
        for (layout, shown_by) in [(Layout::Split, &split), (Layout::Unified, &unified)] {
            let (row_before, in_view_before) =
                pane.read_with(&cx, |pane, _| (pane.position_row(), pane.rows_in_view()));
            let row_before = row_before.expect("the line of the reviewer has a row");
            assert!(in_view_before.contains(&row_before));
            let drawn_before = bounds_of_row(row_before, &mut cx);

            cx.simulate_keystrokes("s");

            assert_eq!(pane.read_with(&cx, |pane, _| pane.layout()), layout);
            assert_eq!(position(pane, &mut cx), line);
            assert_eq!(
                pane.read_with(&cx, |pane, _| pane.text(&line)).as_deref(),
                Some(text)
            );
            let (row, in_view) =
                pane.read_with(&cx, |pane, _| (pane.position_row(), pane.rows_in_view()));
            let row = row.expect("the line of the reviewer has a row");
            assert_eq!(
                rows(pane, &mut cx)[row],
                PaneRow::Diff {
                    path: path.clone(),
                    row: shown_by.clone()
                }
            );
            assert!(row.abs_diff(row_before) > in_view_before.len());
            assert!(in_view.contains(&row), "row {row} is not in {in_view:?}");
            let drawn = bounds_of_row(row, &mut cx);
            assert!(drawn.top() >= px(0.) && drawn.bottom() <= WINDOW.height);
            assert_eq!(drawn.top(), drawn_before.top());
            assert_lines_drawn_as_read(pane, &mut cx);
        }
    }

    let line = position(pane, &mut cx);
    for layout in [Layout::Split, Layout::Unified] {
        let row_before = pane
            .read_with(&cx, |pane, _| pane.position_row())
            .expect("the line of the reviewer has a row");
        let over_the_rows = bounds_of_row(row_before, &mut cx).center();
        cx.simulate_event(ScrollEvent {
            position: over_the_rows,
            delta: ScrollDelta::Pixels(point(px(0.), px(4000.))),
            ..ScrollEvent::default()
        });
        cx.run_until_parked();
        let in_view_before = pane.read_with(&cx, |pane, _| pane.rows_in_view());
        assert_eq!(in_view_before.start, 0);
        assert!(!in_view_before.contains(&row_before));
        assert_eq!(drawn(&format!("review-row-{row_before}"), &mut cx), None);
        assert_eq!(position(pane, &mut cx), line);

        cx.simulate_keystrokes("s");

        assert_eq!(pane.read_with(&cx, |pane, _| pane.layout()), layout);
        assert_eq!(position(pane, &mut cx), line);
        let (row, in_view) =
            pane.read_with(&cx, |pane, _| (pane.position_row(), pane.rows_in_view()));
        let row = row.expect("the line of the reviewer has a row");
        assert_ne!(row, row_before);
        assert!(in_view.contains(&row), "row {row} is not in {in_view:?}");
        let drawn = bounds_of_row(row, &mut cx);
        assert!(drawn.top() >= px(0.) && drawn.bottom() <= WINDOW.height);
    }
}

fn lines_of(path: &str, pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> [Vec<String>; 2] {
    pane.read_with(cx, |pane, _| {
        [Side::Old, Side::New].map(|side| {
            (0..)
                .map_while(|line| {
                    pane.text(&Position {
                        path: PathBuf::from(path),
                        side,
                        line,
                    })
                })
                .collect()
        })
    })
}

fn lines(version: &str) -> Vec<String> {
    version.lines().map(str::to_owned).collect()
}

fn a_thread(digit: char, path: &str, line: u32, state: ThreadState) -> Thread {
    let comment = Comment {
        id: CommentId::new(digit.to_string().repeat(32)),
        author: Author::Reviewer,
        text: format!("comment {digit}"),
        round: (state != ThreadState::Draft).then_some(1),
    };
    Thread {
        id: ThreadId::new(digit.to_string().repeat(32)),
        anchor: Anchor {
            tree: None,
            path: PathBuf::from(path),
            side: LinkSide::New,
            lines: Lines {
                first: line,
                last: line,
            },
            text: format!("line {line}"),
        },
        resolved: state == ThreadState::Resolved,
        addressed: false,
        comments: vec![comment],
        state,
        now: None,
    }
}

#[derive(Clone, Copy)]
enum Effect {
    Moves,
    Notice,
    NothingWaits,
    Setting,
    Box,
    Field,
    Sends(RequestKind),
    Views,
    Follow,
    Form,
    Resume,
    Nothing,
}

fn effect_of(keystroke: &str) -> Effect {
    match keystroke {
        "j" | "down" | "k" | "up" | "s" => Effect::Moves,
        "c" | "o" => Effect::Box,
        "shift-c" => Effect::Field,
        // No history to take the focus to, no turn offered anything, no other session has a review.
        "i" | "d d" | "m" | "shift-m" | "tab" | "shift-o" | "shift-a" => Effect::Notice,
        // None moved to code that changed, no round has a state, no agent to start or choose.
        "w" | "shift-s" | "shift-n" | "ctrl-n" => Effect::Notice,
        "t" => Effect::Sends(RequestKind::AttachReview),
        "x" => Effect::Sends(RequestKind::ResolveThread),
        "u" => Effect::Sends(RequestKind::ReopenThread),
        "shift-t" => Effect::Setting,
        "f" => Effect::Views,
        "shift-f" => Effect::Follow,
        "shift-l" => Effect::Form,
        "shift-g" => Effect::Resume,
        "l" | "h" => Effect::Moves,
        // No Navigator is connected, and the notice says so.
        "shift-k" | "g d" | "g r" => Effect::Notice,
        "escape" => Effect::Nothing,
        // No language server is known, so nothing waits, and each key says so in the notice.
        // `tests/servers.rs` shows with its barrier that nothing is sent.
        "shift-i" | "shift-u" | "shift-y" | "shift-q" => Effect::NothingWaits,
        "y" => Effect::Sends(RequestKind::CopyReview),
        "r" => Effect::Sends(RequestKind::SetViewed),
        "shift-r" => Effect::Sends(RequestKind::MarkReviewed),
        "a" => Effect::Sends(RequestKind::AttachReview),
        "shift-d" => Effect::Sends(RequestKind::DetachReview),
        "shift-x" => Effect::Sends(RequestKind::ResolveAddressed),
        other => panic!("the test names no effect of the key {other}"),
    }
}

impl Effect {
    fn shown(self, stage: &mut Stage, title: &str, how: &str, cx: &mut VisualTestContext) {
        let pane = stage.pane.clone();
        match self {
            Effect::Moves => {}
            Effect::Notice => assert!(
                pane.read_with(cx, |pane, _| pane.notice().is_some()),
                "{title} by {how} said nothing"
            ),
            Effect::NothingWaits => assert!(
                pane.read_with(cx, |pane, _| pane.notice())
                    .is_some_and(|notice| notice.starts_with("No ") && notice.contains(" wait")),
                "{title} by {how} did not say that nothing waits"
            ),
            Effect::Setting => assert_eq!(
                pane.read_with(cx, |pane, _| pane.keeps_selection()),
                how == "its key",
                "{title} by {how} did not set the pane"
            ),
            Effect::Views => assert_eq!(
                pane.read_with(cx, |pane, _| pane.view()),
                if how == "its key" {
                    View::Flow
                } else {
                    View::Changes
                },
                "{title} by {how} showed the wrong view"
            ),
            Effect::Follow => assert_eq!(
                pane.read_with(cx, |pane, _| pane.following()),
                if how == "its key" {
                    Following::Off
                } else {
                    Following::On
                },
                "{title} by {how} left follow wrong"
            ),
            Effect::Form => assert_eq!(
                pane.read_with(cx, |pane, _| pane.flow_form()),
                if how == "its key" {
                    FlowForm::LiveDiff
                } else {
                    FlowForm::Timeline
                },
                "{title} by {how} showed the wrong form"
            ),
            Effect::Resume => assert_eq!(
                pane.read_with(cx, |pane, _| pane.notice()).as_deref(),
                Some("Follow is not paused"),
                "{title} by {how} did not say that nothing is paused"
            ),
            Effect::Nothing => assert!(
                pane.read_with(cx, |pane, _| {
                    pane.notice().is_none()
                        && pane.file_view().is_none()
                        && pane.hover().is_none()
                        && pane.references().is_none()
                        && pane.comment_box().is_none()
                }),
                "{title} by {how} said or showed something"
            ),
            Effect::Box => assert!(
                pane.read_with(cx, |pane, _| pane.comment_box().is_some()),
                "{title} by {how} opened no box"
            ),
            Effect::Field => assert!(
                pane.read_with(cx, |pane, _| pane.commit_field_open()),
                "{title} by {how} opened no field"
            ),
            Effect::Sends(kind) => {
                let (to, request) = stage.keeper.request(cx);
                assert_eq!(request.kind(), kind, "{title} by {how} asked {request:?}");
                let reply = match kind {
                    RequestKind::CopyReview => Err(Refusal::NothingToSend {
                        review: common::review_id(),
                    }),
                    RequestKind::AttachReview => Ok(Reply::AttachedReview { start: Vec::new() }),
                    _ => Ok(Reply::Done),
                };
                stage.keeper.reply(to, reply);
            }
        }
    }
}

// This test shows an absence, so it is only as strong as its list of keys: every key and action
// of the pane's table, and the keys an editor takes as an edit.
#[gpui_kit::test]
fn no_key_and_no_action_changes_the_content_of_a_file_the_pane_shows(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open_staged(
        vec![workspace('1', "feature")],
        |keeper| {
            let mut review = common::no_agent_review();
            review.threads = vec![
                a_thread('1', "story.txt", 3, ThreadState::Sent),
                a_thread('2', "story.txt", 3, ThreadState::Resolved),
            ];
            keeper.set_reviews(reviews_of(vec![review]));
        },
        cx,
    );
    let changes = since_main(
        vec![commit('b', "Edit the story")],
        vec![
            file("story.txt", FileStatus::Modified),
            file("fresh.txt", FileStatus::Added),
        ],
    );
    let story = ("once\nupon\na time\n", "once\nupon\na midnight\ndreary\n");
    let fresh = "all new\nlines\n";
    stage.keeper.show(
        &changes,
        |file| match file.path.to_str() {
            Some("story.txt") => texts(story.0, story.1),
            _ => Ok(FileSides {
                old: Content::Absent,
                new: Content::Text(fresh.as_bytes().to_vec()),
            }),
        },
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the threads of the review", |cx| {
        rows(&stage.pane, cx)
            .iter()
            .any(|row| matches!(row, PaneRow::Thread { .. }))
    });
    let shown = |pane: &Entity<ReviewPane>, cx: &mut TestAppContext| {
        (
            lines_of("story.txt", pane, cx),
            lines_of("fresh.txt", pane, cx),
        )
    };
    let before = shown(&stage.pane, &mut cx);
    assert_eq!(before.0, [lines(story.0), lines(story.1)]);
    assert_eq!(before.1, [Vec::new(), lines(fresh)]);

    let a_line = bounds_of_row(5, &mut cx).center();
    cx.simulate_click(a_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    cx.write_to_clipboard(ClipboardItem::new_string("pasted".to_owned()));

    let the_pane = stage.pane.clone();
    let state = |cx: &mut VisualTestContext| {
        the_pane.read_with(cx, |pane, _| {
            (pane.layout(), pane.position().cloned(), pane.name_at())
        })
    };
    // The keys of the history tree and of the references list act only with the focus there, and
    // are pressed where they work by the tests of those places.
    for control in controls()
        .into_iter()
        .filter(|control| control.place != Place::In(HISTORY_CONTEXT))
        .filter(|control| control.place != Place::In(REFERENCES_CONTEXT))
    {
        let on_the_line = Position {
            path: PathBuf::from("story.txt"),
            side: Side::New,
            line: 2,
        };
        if the_pane.read_with(&cx, |pane, _| pane.position().cloned()) != Some(on_the_line) {
            let a_line = bounds_of_row(5, &mut cx).center();
            cx.simulate_click(a_line, Modifiers::none());
        }
        let effect = effect_of(control.keystroke);
        // `shift-l` and `shift-g` act only while Flow is on screen: with Changes shown they change
        // and send nothing, which is checked first, and then Flow is shown and their effect is
        // checked.
        let only_in_flow = matches!(effect, Effect::Form | Effect::Resume);
        if only_in_flow {
            let seen = |cx: &mut VisualTestContext| {
                the_pane.read_with(cx, |pane, _| {
                    (
                        pane.view(),
                        pane.flow_form(),
                        pane.following(),
                        pane.notice(),
                    )
                })
            };
            let before = seen(&mut cx);
            assert_eq!(before.0, View::Changes);
            cx.simulate_keystrokes(control.keystroke);
            assert_eq!(seen(&mut cx), before, "{} acted in Changes", control.title);
            cx.update(|window, cx| window.dispatch_action((control.action)(), cx));
            cx.run_until_parked();
            assert_eq!(
                seen(&mut cx),
                before,
                "{} acted in Changes as an action",
                control.title
            );
            let to_flow = drawn("review-view-flow", &mut cx)
                .expect("the bar is drawn")
                .center();
            cx.simulate_click(to_flow, Modifiers::none());
            cx.run_until_parked();
            assert_eq!(the_pane.read_with(&cx, |pane, _| pane.view()), View::Flow);
        }
        let at_first = state(&mut cx);
        cx.simulate_keystrokes(control.keystroke);
        let after_the_key = state(&mut cx);
        assert_eq!(
            shown(&stage.pane, &mut cx),
            before,
            "after the key {}",
            control.keystroke
        );
        effect.shown(&mut stage, control.title, "its key", &mut cx);
        // The key's notice or open box would show for the action too; it is taken away so that what
        // shows after is the action's own.
        match effect {
            Effect::Box | Effect::Field => cx.simulate_keystrokes("escape"),
            Effect::Notice | Effect::NothingWaits | Effect::Resume => cx.simulate_keystrokes("s s"),
            Effect::Moves
            | Effect::Nothing
            | Effect::Setting
            | Effect::Sends(_)
            | Effect::Views
            | Effect::Follow
            | Effect::Form => {}
        }
        if matches!(
            effect,
            Effect::Notice | Effect::NothingWaits | Effect::Resume | Effect::Box | Effect::Field
        ) {
            assert!(
                stage.pane.read_with(&cx, |pane, _| pane.notice().is_none()
                    && pane.comment_box().is_none()
                    && !pane.commit_field_open()),
                "{} left something behind that the action would show",
                control.title
            );
        }
        cx.update(|window, cx| window.dispatch_action((control.action)(), cx));
        cx.run_until_parked();
        let after_the_action = state(&mut cx);
        assert_eq!(
            shown(&stage.pane, &mut cx),
            before,
            "after the action {}",
            control.title
        );
        if matches!(effect, Effect::Moves) {
            assert!(
                at_first != after_the_key || after_the_key != after_the_action,
                "{} did nothing",
                control.title
            );
        } else {
            effect.shown(&mut stage, control.title, "the action", &mut cx);
        }
        if matches!(effect, Effect::Nothing) {
            assert_eq!(
                at_first, after_the_key,
                "{} moved the reviewer",
                control.title
            );
            assert_eq!(
                after_the_key, after_the_action,
                "{} as an action",
                control.title
            );
        }
        let turn_end = |selects| Setting::TurnEnd(selects);
        if matches!(effect, Effect::Setting) {
            wait_until(
                &mut cx,
                "the two sets of the turn end to reach the Keeper",
                |_| {
                    stage.keeper.settings_set()
                        == [
                            turn_end(TurnEndSelects::KeepSelection),
                            turn_end(TurnEndSelects::WhatChanged),
                        ]
                },
            );
        }
        if matches!(effect, Effect::Form) {
            let form = |form| Setting::FlowForm {
                workspace: id('1'),
                form,
            };
            wait_until(&mut cx, "the two forms to reach the Keeper", |_| {
                stage.keeper.settings_set()
                    == [
                        turn_end(TurnEndSelects::KeepSelection),
                        turn_end(TurnEndSelects::WhatChanged),
                        form(FlowForm::LiveDiff),
                        form(FlowForm::Timeline),
                    ]
            });
        }
        if only_in_flow {
            let to_changes = drawn("review-view-changes", &mut cx)
                .expect("the bar is drawn")
                .center();
            cx.simulate_click(to_changes, Modifiers::none());
            cx.run_until_parked();
            assert_eq!(
                the_pane.read_with(&cx, |pane, _| pane.view()),
                View::Changes
            );
        }
        // A box or field a key opened takes the following keys as its text.
        if stage.pane.read_with(&cx, |pane, _| {
            pane.comment_box().is_some() || pane.commit_field_open()
        }) {
            cx.simulate_keystrokes("escape");
        }
    }
    let letters_and_digits: Vec<String> = ('a'..='z')
        .chain('0'..='9')
        .map(|key| key.to_string())
        .collect();
    let editing = [
        "shift-a",
        "backspace",
        "delete",
        "enter",
        "space",
        "tab",
        "escape",
        "cmd-x",
        "cmd-v",
        "cmd-z",
        "cmd-shift-z",
        "cmd-s",
        "ctrl-x",
        "ctrl-v",
        "ctrl-z",
        "ctrl-s",
        "shift-insert",
        "shift-delete",
    ];
    for key in letters_and_digits.iter().map(String::as_str).chain(editing) {
        cx.simulate_keystrokes(key);
        assert_eq!(shown(&stage.pane, &mut cx), before, "after the key {key}");
    }
    cx.simulate_input("typed");

    assert_eq!(shown(&stage.pane, &mut cx), before);

    // The keys' requests were written before each key returned, so they precede the question for a
    // workspace the Window has not heard of. The workspace on screen would not do: a key that asked
    // for its changes again would end the reading early.
    stage
        .keeper
        .tell(Fact::Workspaces(vec![repository(vec![workspace(
            '2', "other",
        )])]));
    let closing = Request::GetChanges { workspace: id('2') };
    loop {
        let (_, request) = stage.keeper.request(&mut cx);
        // These only read or change what the Keeper keeps of a review, none writes a worktree file.
        // The commit is not asked: Escape closes its field before a key can send it.
        assert!(
            matches!(
                request.kind(),
                RequestKind::GetChanges
                    | RequestKind::ReadFile
                    | RequestKind::SaveDraft
                    | RequestKind::DeleteDraft
                    | RequestKind::CopyReview
                    | RequestKind::MarkReviewed
                    | RequestKind::SetViewed
                    | RequestKind::AttachReview
                    | RequestKind::DetachReview
                    | RequestKind::ResolveThread
                    | RequestKind::ReopenThread
                    | RequestKind::ResolveAddressed
            ),
            "a key made the Window ask {request:?}"
        );
        if request == closing {
            break;
        }
    }
}

#[gpui_kit::test]
fn j_and_k_walk_every_line_of_the_diff_in_one_order_whatever_the_layout(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        Vec::new(),
        vec![
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
        ],
    );
    stage.keeper.show(
        &changes,
        |file| match file.path.to_str() {
            Some("a.txt") => texts(
                "keep\nold 1\nold 2\nend\n",
                "keep\nnew 1\nnew 2\nnew 3\nend\n",
            ),
            _ => Ok(FileSides {
                old: Content::Absent,
                new: Content::Text(b"only\n".to_vec()),
            }),
        },
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    let pane = &stage.pane;
    let at = |path: &str, side, line| Position {
        path: PathBuf::from(path),
        side,
        line,
    };
    let order = [
        at("a.txt", Side::New, 0),
        at("a.txt", Side::Old, 1),
        at("a.txt", Side::Old, 2),
        at("a.txt", Side::New, 1),
        at("a.txt", Side::New, 2),
        at("a.txt", Side::New, 3),
        at("a.txt", Side::New, 4),
        at("b.txt", Side::New, 0),
    ];
    assert_eq!(position(pane, &mut cx), order[1]);

    cx.simulate_keystrokes("j");
    assert_eq!(position(pane, &mut cx), order[1]);
    let first = bounds_of_row(1, &mut cx).center();
    cx.simulate_click(first, Modifiers::none());
    assert_eq!(position(pane, &mut cx), order[0]);

    for (down, up) in [("j", "k"), ("down", "up")] {
        for layout in [Layout::Unified, Layout::Split] {
            assert_eq!(pane.read_with(&cx, |pane, _| pane.layout()), layout);
            let mut walked = vec![position(pane, &mut cx)];
            for _ in 1..order.len() {
                cx.simulate_keystrokes(down);
                walked.push(position(pane, &mut cx));
            }
            assert_eq!(walked, order, "{down} in {layout:?}");
            let (row, shown) = pane.read_with(&cx, |pane, _| {
                let row = pane.position_row().expect("a row for the last line");
                (row, pane.rows()[row].clone())
            });
            assert_eq!(
                shown,
                PaneRow::Diff {
                    path: PathBuf::from("b.txt"),
                    row: match layout {
                        Layout::Unified => Row::Added { new: 0 },
                        Layout::Split => Row::Changed {
                            old: None,
                            new: Some(0)
                        },
                    }
                },
                "row {row}"
            );
            cx.simulate_keystrokes(down);
            assert_eq!(position(pane, &mut cx), order[order.len() - 1]);
            for back in order.iter().rev().skip(1) {
                cx.simulate_keystrokes(up);
                assert_eq!(position(pane, &mut cx), *back, "{up} in {layout:?}");
            }
            cx.simulate_keystrokes(up);
            assert_eq!(position(pane, &mut cx), order[0]);
            cx.simulate_keystrokes("s");
        }
    }
}

#[gpui_kit::test]
fn unchanged_lines_far_from_any_change_are_one_row_that_the_reviewer_steps_over(
    cx: &mut TestAppContext,
) {
    let version = |word: &str| {
        (0..30)
            .map(|line| match line {
                10 | 20 => format!("{word} {line}\n"),
                _ => format!("line {line}\n"),
            })
            .collect::<String>()
    };
    let (stage, path, mut cx) = open_on_one_file(&version("old"), &version("new"), cx);
    let pane = &stage.pane;
    assert_eq!(
        reading(pane, &mut cx),
        [
            "# src/long.txt",
            "… 7",
            " line 7",
            " line 8",
            " line 9",
            "-old 10",
            "+new 10",
            " line 11",
            " line 12",
            " line 13",
            "… 3",
            " line 17",
            " line 18",
            " line 19",
            "-old 20",
            "+new 20",
            " line 21",
            " line 22",
            " line 23",
            "… 6",
        ]
    );
    let at = |line| Position {
        path: path.clone(),
        side: Side::New,
        line,
    };
    let click = |row: usize, cx: &mut VisualTestContext| {
        let middle = bounds_of_row(row, cx).center();
        cx.simulate_click(middle, Modifiers::none());
    };
    // Opening the pane clicked the row of the lines left out, which is no line: the reviewer stays
    // on the first changed line, where they were put at first.
    assert_eq!(
        position(pane, &mut cx),
        Position {
            path: path.clone(),
            side: Side::Old,
            line: 10
        }
    );
    click(2, &mut cx);
    assert_eq!(position(pane, &mut cx), at(7));
    cx.simulate_keystrokes("k");
    assert_eq!(position(pane, &mut cx), at(7));
    click(9, &mut cx);
    assert_eq!(position(pane, &mut cx), at(13));
    click(10, &mut cx);
    assert_eq!(position(pane, &mut cx), at(13));
    cx.simulate_keystrokes("j");
    assert_eq!(position(pane, &mut cx), at(17));
    cx.simulate_keystrokes("k");
    assert_eq!(position(pane, &mut cx), at(13));
    cx.simulate_keystrokes("s j");
    assert_eq!(position(pane, &mut cx), at(17));
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane.text(&at(15))).as_deref(),
        Some("line 15")
    );
}

#[gpui_kit::test]
fn a_click_on_a_line_puts_the_reviewer_on_it(cx: &mut TestAppContext) {
    let (stage, path, mut cx) = open_on_one_file("keep\nold\nend\n", "keep\nnew\nend\n", cx);
    let pane = &stage.pane;
    let at = |side, line| Position {
        path: path.clone(),
        side,
        line,
    };
    let click = |row: usize, cx: &mut VisualTestContext| {
        let middle = bounds_of_row(row, cx).center();
        cx.simulate_click(middle, Modifiers::none());
    };
    // Rows: the heading, then keep, -old, +new, end.
    click(3, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::New, 1));
    click(2, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::Old, 1));
    click(4, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::New, 2));
    click(0, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::New, 2));

    // Split view: the heading, then keep, old beside new, end.
    cx.simulate_keystrokes("s");
    assert_eq!(
        rows(pane, &mut cx)[2],
        PaneRow::Diff {
            path: path.clone(),
            row: Row::Changed {
                old: Some(1),
                new: Some(1)
            }
        }
    );
    for (half, line) in [
        ("review-row-2-old", at(Side::Old, 1)),
        ("review-row-2-new", at(Side::New, 1)),
    ] {
        let half = drawn(half, &mut cx).expect("the half of the row is laid out");
        cx.simulate_click(half.center(), Modifiers::none());
        assert_eq!(position(pane, &mut cx), line);
    }
    for half in ["review-row-1-old", "review-row-1-new"] {
        let half = drawn(half, &mut cx).expect("the half of the row is laid out");
        cx.simulate_click(half.center(), Modifiers::none());
        assert_eq!(position(pane, &mut cx), at(Side::New, 0));
    }
}

#[gpui_kit::test]
fn of_a_long_diff_only_the_rows_in_view_are_laid_out_and_the_wheel_moves_the_view_not_the_reviewer(
    cx: &mut TestAppContext,
) {
    let new: String = (0..5000).map(|line| format!("line {line}\n")).collect();
    let (stage, path, mut cx) = open_on_one_file("", &new, cx);
    let height = WINDOW.height + px(20.);
    cx.simulate_resize(size(WINDOW.width, height));
    let pane = &stage.pane;
    assert_eq!(rows(pane, &mut cx).len(), 5001);

    let in_view = |cx: &mut VisualTestContext| pane.read_with(cx, |pane, _| pane.rows_in_view());
    let at_first = in_view(&mut cx);
    assert_eq!(at_first.start, 0);
    assert!(
        at_first.len() > 10 && at_first.len() < 30,
        "only the viewport's rows are laid out: {at_first:?}"
    );
    assert!(drawn(&format!("review-row-{}", at_first.end - 1), &mut cx).is_some());
    assert_eq!(
        drawn(&format!("review-row-{}", at_first.end), &mut cx),
        None
    );
    assert_eq!(drawn("review-row-5000", &mut cx), None);

    let on = position(pane, &mut cx);
    let over_the_rows = bounds_of_row(5, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    let scrolled = in_view(&mut cx);
    assert!(
        scrolled.start >= 95 && scrolled.start <= 105,
        "{scrolled:?}"
    );
    assert_eq!(
        drawn(&format!("review-row-{}", at_first.end - 1), &mut cx),
        None
    );
    assert!(drawn(&format!("review-row-{}", scrolled.start), &mut cx).is_some());
    assert_eq!(position(pane, &mut cx), on);
    assert_eq!(
        on,
        Position {
            path: path.clone(),
            side: Side::New,
            line: 0
        }
    );

    cx.simulate_keystrokes("j");
    assert_eq!(
        position(pane, &mut cx),
        Position {
            path,
            side: Side::New,
            line: 1
        }
    );
    let row = pane.read_with(&cx, |pane, _| pane.position_row());
    assert_eq!(row, Some(2));
    assert!(in_view(&mut cx).contains(&2));

    for _ in 0..40 {
        cx.simulate_keystrokes("j");
    }
    let row = pane
        .read_with(&cx, |pane, _| pane.position_row())
        .expect("a row");
    assert_eq!(row, 42);
    assert!(in_view(&mut cx).contains(&row));
    let drawn = bounds_of_row(row, &mut cx);
    assert!(drawn.top() >= px(0.) && drawn.bottom() <= height);
}

#[gpui_kit::test]
fn a_file_with_no_lines_to_show_says_why_and_a_file_with_one_version_is_all_removed_or_all_added(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = link_up(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        Vec::new(),
        vec![
            file("art.png", FileStatus::Modified),
            file("dump.sql", FileStatus::Modified),
            file("gone.txt", FileStatus::Deleted),
            file(
                "moved.txt",
                FileStatus::Renamed {
                    from: PathBuf::from("was.txt"),
                },
            ),
            file("secret.txt", FileStatus::Modified),
        ],
    );
    let asked = keeper.expect(Request::GetChanges { workspace: id('1') }, cx);
    keeper.reply(asked, Ok(Reply::Changes(changes.clone())));
    wait_until(cx, "the changes in the shared model", |cx| {
        models
            .changes
            .read_with(cx, |model, _| model.changes().is_some())
    });
    let (mut stage, mut cx) = open_window(models, keeper, cx);
    for file in &changes.files {
        let asked = stage.keeper.expect(read_file(&changes, file), &mut cx);
        let answer = match file.path.to_str().expect("a path of the test") {
            "art.png" => Ok(FileSides {
                old: Content::Binary { size: 2048 },
                new: Content::Binary { size: 4096 },
            }),
            "dump.sql" => Ok(FileSides {
                old: Content::Text(b"small\n".to_vec()),
                new: Content::TooLarge { size: 5 << 20 },
            }),
            "gone.txt" => Ok(FileSides {
                old: Content::Text(b"first\nsecond\n".to_vec()),
                new: Content::Absent,
            }),
            "moved.txt" => texts("same\nold\n", "same\nnew\n"),
            _ => Err(Refusal::Failed {
                reason: "the file cannot be read".to_owned(),
            }),
        };
        stage.keeper.reply(asked, answer.map(Reply::File));
    }
    wait_for_files(&stage.pane, &mut cx);

    let heading = |path: &str, status: FileStatus| PaneRow::File {
        path: PathBuf::from(path),
        status,
    };
    let unshown = |path: &str, why| PaneRow::Unshown {
        path: PathBuf::from(path),
        why,
    };
    let diff = |path: &str, row| PaneRow::Diff {
        path: PathBuf::from(path),
        row,
    };
    assert_eq!(
        rows(&stage.pane, &mut cx),
        [
            heading("art.png", FileStatus::Modified),
            unshown("art.png", Unshown::Binary),
            heading("dump.sql", FileStatus::Modified),
            unshown("dump.sql", Unshown::TooLarge),
            heading("gone.txt", FileStatus::Deleted),
            diff("gone.txt", Row::Removed { old: 0 }),
            diff("gone.txt", Row::Removed { old: 1 }),
            heading(
                "moved.txt",
                FileStatus::Renamed {
                    from: PathBuf::from("was.txt")
                }
            ),
            diff("moved.txt", Row::Context { old: 0, new: 0 }),
            diff("moved.txt", Row::Removed { old: 1 }),
            diff("moved.txt", Row::Added { new: 1 }),
            heading("secret.txt", FileStatus::Modified),
            unshown("secret.txt", Unshown::Refused),
        ]
    );
    assert_eq!(
        reading(&stage.pane, &mut cx),
        [
            "# art.png",
            "# dump.sql",
            "# gone.txt",
            "-first",
            "-second",
            "# moved.txt",
            " same",
            "-old",
            "+new",
            "# secret.txt",
        ]
    );
    stage.pane.read_with(&cx, |pane, _| {
        for path in ["art.png", "dump.sql", "secret.txt"] {
            for side in [Side::Old, Side::New] {
                let first = Position {
                    path: PathBuf::from(path),
                    side,
                    line: 0,
                };
                assert_eq!(pane.text(&first), None, "{path}");
            }
        }
    });
    assert_eq!(
        position(&stage.pane, &mut cx),
        Position {
            path: PathBuf::from("gone.txt"),
            side: Side::Old,
            line: 0
        }
    );
    cx.simulate_resize(size(WINDOW.width, px(600.)));
    for row in 0..13 {
        bounds_of_row(row, &mut cx);
    }
}

#[gpui_kit::test]
fn a_line_is_held_as_the_file_has_it_whatever_its_bytes_and_however_long_it_is(
    cx: &mut TestAppContext,
) {
    // A long line ending in a changed word of 3-byte characters that begins a few bytes before the
    // cut at 1,024 bytes, so its mark has to end between two characters there.
    let wide = |last_word: &str| format!("{} {last_word}", "x".repeat(1019));
    let old = format!("naïve ✓ 日本\n{}\n", wide("日本語日本語"));
    let new = format!("naïve ✗ 日本\n{}\n", wide("日本語日本"));
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        Vec::new(),
        vec![
            file("latin.txt", FileStatus::Added),
            file("words.txt", FileStatus::Modified),
        ],
    );
    stage.keeper.show(
        &changes,
        |file| match file.path.to_str() {
            Some("latin.txt") => Ok(FileSides {
                old: Content::Absent,
                new: Content::Text(b"caf\xe9\n".to_vec()),
            }),
            _ => texts(&old, &new),
        },
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);

    assert_eq!(
        reading(&stage.pane, &mut cx),
        [
            "# latin.txt",
            "+caf\u{fffd}",
            "# words.txt",
            "-naïve ✓ 日本",
            format!("-{}", wide("日本語日本語")).as_str(),
            "+naïve ✗ 日本",
            format!("+{}", wide("日本語日本")).as_str(),
        ]
    );
    for row in 0..7 {
        bounds_of_row(row, &mut cx);
    }
    // Of the long line only its start is drawn: up to the last whole character within 1,024 bytes.
    assert!(drawn("review-text-1-new:caf\u{fffd}", &mut cx).is_some());
    assert!(drawn("review-text-3-old:naïve ✓ 日本", &mut cx).is_some());
    assert!(drawn("review-text-5-new:naïve ✗ 日本", &mut cx).is_some());
    let start = format!("{} 日", "x".repeat(1019));
    assert_eq!(start.len(), 1023);
    assert!(drawn(&format!("review-text-4-old:{start}"), &mut cx).is_some());
    assert!(drawn(&format!("review-text-6-new:{start}"), &mut cx).is_some());
}

#[gpui_kit::test]
fn changes_read_again_stay_as_they_are_shown_until_the_new_versions_are_here(
    cx: &mut TestAppContext,
) {
    let (mut stage, path, mut cx) = open_on_one_file("keep\nold\nend\n", "keep\nnew\nend\n", cx);
    let changes = since_main(Vec::new(), vec![file("src/long.txt", FileStatus::Modified)]);
    let read = read_file(&changes, &changes.files[0]);
    let removed = Position {
        path: path.clone(),
        side: Side::Old,
        line: 1,
    };
    let on_the_removed_line = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(on_the_removed_line, Modifiers::none());
    assert_eq!(position(&stage.pane, &mut cx), removed);
    let shown = rows(&stage.pane, &mut cx);

    let read_again = |stage: &mut Stage, cx: &mut VisualTestContext| {
        stage
            .keeper
            .tell(Fact::ChangesChanged { workspace: id('1') });
        let asked = stage
            .keeper
            .expect(Request::GetChanges { workspace: id('1') }, cx);
        stage
            .keeper
            .reply(asked, Ok(Reply::Changes(changes.clone())));
        stage.keeper.expect(read.clone(), cx)
    };
    let asked = read_again(&mut stage, &mut cx);
    assert_eq!(rows(&stage.pane, &mut cx), shown);
    assert_eq!(position(&stage.pane, &mut cx), removed);

    stage.keeper.reply(
        asked,
        texts("keep\nold\nend\n", "keep\nnewer\nmore\nend\n").map(Reply::File),
    );
    wait_until(&mut cx, "the new version of the file", |cx| {
        reading(&stage.pane, cx) == ["# src/long.txt", " keep", "-old", "+newer", "+more", " end"]
    });
    assert_eq!(position(&stage.pane, &mut cx), removed);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.position_row()),
        Some(2)
    );

    let asked = read_again(&mut stage, &mut cx);
    stage.keeper.reply(
        asked,
        texts("keep\nold\nend\n", "keep\nold\nend\nlast\n").map(Reply::File),
    );
    wait_until(&mut cx, "the newest version of the file", |cx| {
        reading(&stage.pane, cx) == ["# src/long.txt", " keep", " old", " end", "+last"]
    });
    assert_eq!(
        position(&stage.pane, &mut cx),
        Position {
            path,
            side: Side::New,
            line: 3
        }
    );
}

#[gpui_kit::test]
fn changes_the_keeper_refuses_to_read_again_stay_as_they_are_shown_and_the_pane_says_so(
    cx: &mut TestAppContext,
) {
    let lines = |count: usize| {
        (0..count)
            .map(|line| format!("line {line}\n"))
            .collect::<String>()
    };
    let (mut stage, _, mut cx) = open_on_one_file("", &lines(100), cx);
    let height = WINDOW.height + px(20.);
    cx.simulate_resize(size(WINDOW.width, height));
    let changes = since_main(Vec::new(), vec![file("src/long.txt", FileStatus::Modified)]);
    let shown = rows(&stage.pane, &mut cx);
    assert_eq!(shown.len(), 101);
    let on = position(&stage.pane, &mut cx);
    let in_view =
        |cx: &mut VisualTestContext| stage.pane.read_with(cx, |pane, _| pane.rows_in_view());
    let before = in_view(&mut cx);
    assert_eq!(before.start, 0);
    assert!(before.len() > 10, "{before:?}");
    assert_eq!(drawn("review-refused", &mut cx), None);

    let ask_again = |keeper: &mut common::KeeperEnd, cx: &mut VisualTestContext| {
        keeper.tell(Fact::ChangesChanged { workspace: id('1') });
        keeper.expect(Request::GetChanges { workspace: id('1') }, cx)
    };
    let refused = |cx: &mut TestAppContext| {
        stage
            .models
            .changes
            .read_with(cx, |changes, _| changes.refusal().is_some())
    };
    let asked = ask_again(&mut stage.keeper, &mut cx);
    stage.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "git did not answer".to_owned(),
        }),
    );
    wait_until(&mut cx, "the refusal", refused);

    assert_eq!(rows(&stage.pane, &mut cx), shown);
    assert_eq!(position(&stage.pane, &mut cx), on);
    let said = drawn("review-refused", &mut cx).expect("the pane tells of the refusal");
    assert!(said.top() >= px(0.) && said.bottom() <= bounds_of_row(0, &mut cx).top());
    let with_banner = in_view(&mut cx);
    assert_eq!(with_banner.start, 0);
    assert!(
        with_banner.end < before.end,
        "{with_banner:?} vs {before:?}"
    );
    assert!(bounds_of_row(with_banner.end - 1, &mut cx).top() < height);

    let asked = ask_again(&mut stage.keeper, &mut cx);
    stage
        .keeper
        .reply(asked, Ok(Reply::Changes(changes.clone())));
    let asked = stage
        .keeper
        .expect(read_file(&changes, &changes.files[0]), &mut cx);
    stage
        .keeper
        .reply(asked, texts("", &lines(101)).map(Reply::File));
    wait_until(&mut cx, "the file as it is now", |cx| {
        rows(&stage.pane, cx).len() == 102
    });
    assert!(!refused(&mut cx));
    assert_eq!(drawn("review-refused", &mut cx), None);
    assert_eq!(in_view(&mut cx), before);
}

#[gpui_kit::test]
fn another_workspace_on_screen_takes_the_place_of_the_one_before(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature"), workspace('2', "other")], cx);
    let long = |word: &str| {
        (0..200)
            .map(|line| format!("{word} {line}\n"))
            .collect::<String>()
    };
    let first = since_main(Vec::new(), vec![file("first.txt", FileStatus::Added)]);
    let first_text = long("first");
    stage
        .keeper
        .show(&first, |_| texts("", &first_text), &mut cx);
    wait_for_files(&stage.pane, &mut cx);

    let a_line = bounds_of_row(1, &mut cx).center();
    cx.simulate_click(a_line, Modifiers::none());
    cx.simulate_keystrokes("s");
    for _ in 0..60 {
        cx.simulate_keystrokes("j");
    }
    let in_view = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view());
    assert!(in_view.start > 30, "{in_view:?}");

    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    cx.run_until_parked();
    stage.pane.read_with(&cx, |pane, _| {
        assert_eq!(pane.rows(), []);
        assert_eq!(pane.position(), None);
        assert_eq!(pane.position_row(), None);
        assert_eq!(pane.rows_in_view(), 0..0);
        let of_the_first = Position {
            path: PathBuf::from("first.txt"),
            side: Side::New,
            line: 0,
        };
        assert_eq!(pane.text(&of_the_first), None);
    });

    let second = Changes {
        workspace: id('2'),
        files: (0..40)
            .map(|file_number| file(&format!("second-{file_number:02}.txt"), FileStatus::Added))
            .collect(),
        ..first.clone()
    };
    stage
        .keeper
        .show(&second, |_| texts("", "of the second\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    stage.pane.read_with(&cx, |pane, _| {
        assert_eq!(pane.rows().len(), 80);
        assert_eq!(
            pane.rows()[0],
            PaneRow::File {
                path: PathBuf::from("second-00.txt"),
                status: FileStatus::Added
            }
        );
        assert_eq!(pane.rows_in_view().start, 0);
        let start = Position {
            path: PathBuf::from("second-00.txt"),
            side: Side::New,
            line: 0,
        };
        assert_eq!(pane.position(), Some(&start));
        assert_eq!(pane.text(&start).as_deref(), Some("of the second"));
        assert_eq!(pane.layout(), Layout::Split);
    });
}

#[gpui_kit::test]
fn a_file_that_arrives_above_the_rows_in_view_leaves_them_where_they_are(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        Vec::new(),
        vec![
            file("above.txt", FileStatus::Added),
            file("below.txt", FileStatus::Added),
        ],
    );
    let asked = stage
        .keeper
        .expect(Request::GetChanges { workspace: id('1') }, &mut cx);
    stage
        .keeper
        .reply(asked, Ok(Reply::Changes(changes.clone())));
    let above = stage
        .keeper
        .expect(read_file(&changes, &changes.files[0]), &mut cx);
    let below = stage
        .keeper
        .expect(read_file(&changes, &changes.files[1]), &mut cx);
    let lines = |word: &str, count: usize| {
        (0..count)
            .map(|line| format!("{word} {line}\n"))
            .collect::<String>()
    };
    stage
        .keeper
        .reply(below, texts("", &lines("below", 100)).map(Reply::File));
    wait_until(&mut cx, "the second file", |cx| {
        rows(&stage.pane, cx).len() == 103
    });
    assert_eq!(
        rows(&stage.pane, &mut cx)[1],
        PaneRow::Unshown {
            path: PathBuf::from("above.txt"),
            why: Unshown::Loading
        }
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| pane.position().cloned()),
        None
    );

    let over_the_rows = bounds_of_row(5, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-1000.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    let top = |cx: &mut VisualTestContext| {
        stage.pane.read_with(cx, |pane, _| {
            let first = pane.rows_in_view().start;
            (first, pane.rows()[first].clone())
        })
    };
    let (before_row, read_there) = top(&mut cx);
    assert!(
        matches!(&read_there, PaneRow::Diff { path, .. } if path == &PathBuf::from("below.txt")),
        "scrolled into below.txt, got ({before_row}, {read_there:?})"
    );

    stage
        .keeper
        .reply(above, texts("", &lines("above", 30)).map(Reply::File));
    wait_for_files(&stage.pane, &mut cx);
    assert_eq!(top(&mut cx), (before_row + 29, read_there));
    assert_eq!(
        position(&stage.pane, &mut cx),
        Position {
            path: PathBuf::from("above.txt"),
            side: Side::New,
            line: 0
        }
    );
}

#[gpui_kit::test]
fn of_many_changed_files_only_a_few_are_on_their_way_at_a_time_from_the_top_of_the_list(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let count = 3 * FILES_ON_THEIR_WAY + 1;
    let changes = since_main(
        Vec::new(),
        (0..count)
            .map(|number| file(&format!("file-{number:02}.txt"), FileStatus::Added))
            .collect(),
    );
    let version = |number: usize| texts("", &format!("line of file {number}\n")).map(Reply::File);
    let changed = Fact::ChangesChanged { workspace: id('1') };
    let for_the_changes = Request::GetChanges { workspace: id('1') };
    let asked = stage.keeper.expect(for_the_changes.clone(), &mut cx);
    stage
        .keeper
        .reply(asked, Ok(Reply::Changes(changes.clone())));

    let mut on_their_way = VecDeque::new();
    for file in &changes.files[..FILES_ON_THEIR_WAY] {
        let asked = stage.keeper.expect(read_file(&changes, file), &mut cx);
        on_their_way.push_back(asked);
    }
    // The pane asks for no more files, though none was answered: a question for a further file
    // would come before the one the Keeper makes the Window ask by telling that the changes
    // changed.
    stage.keeper.tell(changed.clone());
    let mut again = stage.keeper.expect(for_the_changes.clone(), &mut cx);

    for number in 0..count {
        let answered = on_their_way
            .pop_front()
            .expect("a question for each file of the list");
        stage.keeper.reply(answered, version(number));
        if let Some(next) = changes.files.get(number + FILES_ON_THEIR_WAY) {
            let asked = stage.keeper.expect(read_file(&changes, next), &mut cx);
            on_their_way.push_back(asked);
        }
        stage.keeper.tell(changed.clone());
        again = stage.keeper.expect(for_the_changes.clone(), &mut cx);
    }
    assert_eq!(on_their_way.len(), 0);
    wait_for_files(&stage.pane, &mut cx);
    let all: Vec<String> = (0..count)
        .flat_map(|number| {
            [
                format!("# file-{number:02}.txt"),
                format!("+line of file {number}"),
            ]
        })
        .collect();
    assert_eq!(reading(&stage.pane, &mut cx), all);

    stage
        .keeper
        .reply(again, Ok(Reply::Changes(changes.clone())));
    for file in &changes.files[..FILES_ON_THEIR_WAY] {
        stage.keeper.expect(read_file(&changes, file), &mut cx);
    }
    stage.keeper.tell(changed);
    stage.keeper.expect(for_the_changes, &mut cx);
}
