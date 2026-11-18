//! Component test: the Review pane on GPUI's test platform, with no window on any screen.
//! Keys, clicks and the wheel go in; the pane is read back through the model it shows.
//! The test plays the Keeper on the other end of a socket pair: it answers what the
//! Window asks for the changes and for each file, and tells it facts.

mod common;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    ClipboardItem, Entity, Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_diff::{Row, Side};
use yaar_frame::SetBase;
use yaar_link::{
    Changes, Comparison, Content, Fact, FileSides, FileStatus, NoComparison, Request, RequestKind,
};
use yaar_review_pane::{Layout, PaneRow, Position, ReviewPane, controls};

use common::{
    Stage, WINDOW, bounds_of_row, commit, default_base, drawn, file, id, open, rows, since_main,
    texts, wait_for_files, wait_until, workspace,
};

/// The rows of the pane as a reader of a unified diff would write them down: a heading
/// for each file, then each line the pane shows with the sign of its row, its text read
/// through the pane.
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
                        // An unchanged line reads the same on both sides.
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
                PaneRow::Comparison { .. } | PaneRow::Commit { .. } | PaneRow::Unshown { .. } => {}
            }
        }
        read
    })
}

// RP-01, first sentence.
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
    // Nothing is shown before the Keeper has answered.
    assert_eq!(rows(&stage.pane, &mut cx), []);

    stage.keeper.show(
        &changes,
        |file| match file.path.to_str() {
            Some("src/numbers.txt") => {
                texts("one\ntwo\nthree\nfour\n", "one\n2\nthree\nfour\nfive\n")
            }
            // A new file has no old version, and the Keeper sends none.
            _ => Ok(FileSides {
                old: Content::Absent,
                new: Content::Text(b"alpha\nbeta\n".to_vec()),
            }),
        },
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);

    // Each file under its heading, each line of its diff as one row, in the order of
    // the file.
    let diff = |path: &PathBuf, row: Row| PaneRow::Diff {
        path: path.clone(),
        row,
    };
    let shown = rows(&stage.pane, &mut cx);
    assert_eq!(
        shown[3..],
        [
            PaneRow::File {
                path: numbers.clone(),
                status: FileStatus::Modified
            },
            diff(&numbers, Row::Context { old: 0, new: 0 }),
            diff(&numbers, Row::Removed { old: 1 }),
            diff(&numbers, Row::Added { new: 1 }),
            diff(&numbers, Row::Context { old: 2, new: 2 }),
            diff(&numbers, Row::Context { old: 3, new: 3 }),
            diff(&numbers, Row::Added { new: 4 }),
            PaneRow::File {
                path: notes.clone(),
                status: FileStatus::Added
            },
            diff(&notes, Row::Added { new: 0 }),
            diff(&notes, Row::Added { new: 1 }),
        ]
    );
    // The text of every line, as the Keeper sent it.
    assert_eq!(
        reading(&stage.pane, &mut cx),
        [
            "# src/numbers.txt",
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
        // A line the diff does not show is still a line of the file the pane holds.
        assert_eq!(at(&numbers, Side::Old, 3).as_deref(), Some("four"));
        // No text for a line the file does not have, on a side it does not have, or of
        // a file that is not among the changes.
        assert_eq!(at(&numbers, Side::Old, 4), None);
        assert_eq!(at(&notes, Side::Old, 0), None);
        assert_eq!(at(&PathBuf::from("src/other.txt"), Side::New, 0), None);
    });

    // Every row is laid out, one under the other in the order of the model.
    let in_view = stage.pane.read_with(&cx, |pane, _| pane.rows_in_view());
    assert_eq!(in_view, 0..shown.len());
    let tops: Vec<_> = in_view
        .map(|index| bounds_of_row(index, &mut cx).top())
        .collect();
    assert!(tops.is_sorted_by(|above, below| above < below), "{tops:?}");
}

/// Sends the pane other changes of the workspace on screen, the way a Keeper does: it
/// tells that they changed, and answers the question that follows.
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
            matches!(pane.rows().first(), Some(PaneRow::Comparison { comparison: shown, .. })
                if *shown == comparison)
        })
    });
    wait_for_files(&stage.pane, cx);
}

// RP-01, second sentence.
#[gpui_kit::test]
fn above_the_changes_the_pane_shows_the_comparison_and_its_commits_and_offers_a_base_where_there_is_none(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "main")], cx);
    // What the Frame does with the offer is the Frame's; here the action is noted.
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

    // A reply of each comparison that has something to compare with: the first row
    // says which, the commits follow newest first as the Keeper lists them, and only
    // then come the files. No base is offered.
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
            shown[0],
            PaneRow::Comparison {
                comparison: changes.comparison.clone(),
                offers_base: false
            }
        );
        assert_eq!(
            shown[1..=commits],
            changes.commits.iter().map(commit_row).collect::<Vec<_>>()[..]
        );
        assert_eq!(shown[commits + 1], heading);
        assert_eq!(
            drawn("review-offer-base", &mut cx),
            None,
            "{:?}",
            changes.comparison
        );
    }

    // Nothing to compare with, for each reason there is: the first row says so and
    // offers to set a base, no commit is listed, and the files follow, which are the
    // uncommitted changes.
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
            shown[0],
            PaneRow::Comparison {
                comparison: changes.comparison.clone(),
                offers_base: true
            }
        );
        assert_eq!(shown[1], heading, "{why:?}");
        assert_eq!(
            shown[2..],
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

        // The offer is a control in the first row: a click on it runs the action that
        // sets a base.
        let offer = drawn("review-offer-base", &mut cx)
            .unwrap_or_else(|| panic!("no base is offered for {why:?}"));
        assert!(bounds_of_row(0, &mut cx).contains(&offer.center()));
        assert_eq!(asked_for_a_base.get(), clicks);
        cx.simulate_click(offer.center(), Modifiers::none());
        assert_eq!(asked_for_a_base.get(), clicks + 1, "{why:?}");
    }
}

/// A file whose diff is longer than the window, in two changes: forty lines replaced
/// by forty others, then, two unchanged lines further down, one line replaced by one.
/// The first change takes eighty rows in the unified view and forty in the split one,
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

/// A window with the pane showing one changed file, `src/long.txt`, with these two
/// versions; the pane has the focus, as after a click on the first line of the diff.
fn open_on_one_file(
    old: &str,
    new: &str,
    cx: &mut TestAppContext,
) -> (Stage, PathBuf, VisualTestContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(Vec::new(), vec![file("src/long.txt", FileStatus::Modified)]);
    stage.keeper.show(&changes, |_| texts(old, new), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    // The keys of the pane work where the pane has the focus, and a click gives it.
    let first_line = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(first_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    (stage, PathBuf::from("src/long.txt"), cx)
}

fn position(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Position {
    pane.read_with(cx, |pane, _| pane.position().cloned())
        .expect("the reviewer is on a line")
}

/// Presses `j` until the reviewer is on `wanted`.
fn walk_down_to(wanted: &Position, pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) {
    for _ in 0..500 {
        if position(pane, cx) == *wanted {
            return;
        }
        cx.simulate_keystrokes("j");
    }
    panic!("j never came to {wanted:?}");
}

// RP-17.
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
    // A removed line, an added line and an unchanged one, each with the row that
    // shows it in the unified and in the split view.
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
        // Unified to split, and back.
        for (layout, shown_by) in [(Layout::Split, &split), (Layout::Unified, &unified)] {
            let (row_before, in_view_before) =
                pane.read_with(&cx, |pane, _| (pane.position_row(), pane.rows_in_view()));
            let row_before = row_before.expect("the line of the reviewer has a row");
            assert!(in_view_before.contains(&row_before));
            let drawn_before = bounds_of_row(row_before, &mut cx);

            cx.simulate_keystrokes("s");

            // The layout changed, and the reviewer is on the line they were on.
            assert_eq!(pane.read_with(&cx, |pane, _| pane.layout()), layout);
            assert_eq!(position(pane, &mut cx), line);
            assert_eq!(
                pane.read_with(&cx, |pane, _| pane.text(&line)).as_deref(),
                Some(text)
            );
            // Its row is another one now, laid out as the new layout has it.
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
            // The row moved by more rows than the window holds, so it is in view only
            // because the pane scrolled with it: it is among the rows laid out, and
            // drawn inside the window.
            assert!(row.abs_diff(row_before) > in_view_before.len());
            assert!(in_view.contains(&row), "row {row} is not in {in_view:?}");
            let drawn = bounds_of_row(row, &mut cx);
            assert!(drawn.top() >= px(0.) && drawn.bottom() <= WINDOW.height);
            // At the height it was at, so the eye of the reviewer need not look for it.
            assert_eq!(drawn.top(), drawn_before.top());
        }
    }
}

/// The text of every line of both versions of a file, read through the pane.
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

/// A version of a file as the lines it has.
fn lines(version: &str) -> Vec<String> {
    version.lines().map(str::to_owned).collect()
}

// RP-23.
//
// This shows an absence, and is as strong as its list of keys and no stronger: a key
// that is not pressed here could still do what the test says no key does. The list is
// every key and every action of the pane's own table, and the keys an editor would
// take as an edit: letters, digits, Backspace, Delete, Enter, Space, Tab, and the keys
// that cut, paste, undo and save.
#[gpui_kit::test]
fn no_key_and_no_action_changes_the_content_of_a_file_the_pane_shows(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
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
    let shown = |pane: &Entity<ReviewPane>, cx: &mut TestAppContext| {
        (
            lines_of("story.txt", pane, cx),
            lines_of("fresh.txt", pane, cx),
        )
    };
    // What the pane shows is what the Keeper sent, line for line.
    let before = shown(&stage.pane, &mut cx);
    assert_eq!(before.0, [lines(story.0), lines(story.1)]);
    assert_eq!(before.1, [Vec::new(), lines(fresh)]);

    // The keys go to the pane: it has the focus, on a line of the first file.
    let a_line = bounds_of_row(3, &mut cx).center();
    cx.simulate_click(a_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    // Something to paste, should a key paste.
    cx.write_to_clipboard(ClipboardItem::new_string("pasted".to_owned()));

    // Every key of the pane's table, and every action of it run without its key. They
    // do reach the pane: each moves the reviewer or switches the layout, one way or
    // the other.
    let state = |cx: &mut VisualTestContext| {
        stage
            .pane
            .read_with(cx, |pane, _| (pane.layout(), pane.position().cloned()))
    };
    for control in controls() {
        let before = state(&mut cx);
        cx.simulate_keystrokes(control.keystroke);
        let after_the_key = state(&mut cx);
        cx.update(|window, cx| window.dispatch_action((control.action)(), cx));
        cx.run_until_parked();
        let after_the_action = state(&mut cx);
        assert!(
            before != after_the_key || after_the_key != after_the_action,
            "{} did nothing",
            control.title
        );
    }
    // The keys that would edit a text.
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
        // Cut, paste, undo, redo and save, as a Mac and as a Linux desktop have them.
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
    }
    // Text typed as an input method hands it over, not key by key.
    cx.simulate_input("typed");

    // Every line of every file reads as it did.
    assert_eq!(shown(&stage.pane, &mut cx), before);

    // And nothing was asked of the Keeper that could change a file. What the keys
    // caused was written to the socket before each of them returned, so it lies before
    // the question the Window asks when the Keeper now tells it that the changes
    // changed: reading up to that question reads all of it, without waiting for a time
    // to pass.
    stage
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let closing = Request::GetChanges { workspace: id('1') };
    loop {
        let (_, request) = stage.keeper.request(&mut cx);
        // Of the requests of the Link, these two only read.
        assert!(
            matches!(
                request.kind(),
                RequestKind::GetChanges | RequestKind::ReadFile
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
            // Two lines replaced by three: in the split view the third row of the
            // change has no old side.
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
    // Every line the pane shows, in the order of the unified view, across both files.
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
    // At first the reviewer is on the first changed line of the first file.
    assert_eq!(position(pane, &mut cx), order[1]);

    // While the pane has not the focus, its keys are not its keys.
    cx.simulate_keystrokes("j");
    assert_eq!(position(pane, &mut cx), order[1]);
    // A click gives it the focus; this one is on the first line of the diff.
    let first = bounds_of_row(2, &mut cx).center();
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
            // The row of the line shows the line, in either layout.
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
            // There is no line after the last, and none before the first.
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
fn a_click_on_a_line_puts_the_reviewer_on_it(cx: &mut TestAppContext) {
    let (stage, path, mut cx) = open_on_one_file("keep\nold\nend\n", "keep\nnew\nend\n", cx);
    let pane = &stage.pane;
    let at = |side, line| Position {
        path: path.clone(),
        side,
        line,
    };
    // The rows: the comparison, the heading, then keep, -old, +new, end.
    let click = |row: usize, cx: &mut VisualTestContext| {
        let middle = bounds_of_row(row, cx).center();
        cx.simulate_click(middle, Modifiers::none());
    };
    click(4, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::New, 1));
    click(3, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::Old, 1));
    click(5, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::New, 2));
    // A row that shows no line leaves the reviewer where they are.
    click(1, &mut cx);
    click(0, &mut cx);
    assert_eq!(position(pane, &mut cx), at(Side::New, 2));

    // In the split view a row of a change shows two lines, and the click says which.
    cx.simulate_keystrokes("s");
    // The rows: the comparison, the heading, then keep, old beside new, end.
    assert_eq!(
        rows(pane, &mut cx)[3],
        PaneRow::Diff {
            path: path.clone(),
            row: Row::Changed {
                old: Some(1),
                new: Some(1)
            }
        }
    );
    for (half, line) in [
        ("review-row-3-old", at(Side::Old, 1)),
        ("review-row-3-new", at(Side::New, 1)),
    ] {
        let half = drawn(half, &mut cx).expect("the half of the row is laid out");
        cx.simulate_click(half.center(), Modifiers::none());
        assert_eq!(position(pane, &mut cx), line);
    }
    // An unchanged line is one line, whichever half of its row is clicked.
    for half in ["review-row-2-old", "review-row-2-new"] {
        let half = drawn(half, &mut cx).expect("the half of the row is laid out");
        cx.simulate_click(half.center(), Modifiers::none());
        assert_eq!(position(pane, &mut cx), at(Side::New, 0));
    }
}

#[gpui_kit::test]
fn of_a_long_diff_only_the_rows_in_view_are_laid_out_and_the_wheel_moves_the_view_not_the_reviewer(
    cx: &mut TestAppContext,
) {
    // A new file of five thousand lines: five thousand rows under two.
    let new: String = (0..5000).map(|line| format!("line {line}\n")).collect();
    let (stage, path, mut cx) = open_on_one_file("", &new, cx);
    let pane = &stage.pane;
    assert_eq!(rows(pane, &mut cx).len(), 5002);

    let in_view = |cx: &mut VisualTestContext| pane.read_with(cx, |pane, _| pane.rows_in_view());
    let at_first = in_view(&mut cx);
    assert_eq!(at_first, 0..21, "a window of 420 px holds 21 rows of 20 px");
    assert!(drawn("review-row-20", &mut cx).is_some());
    assert_eq!(drawn("review-row-21", &mut cx), None);
    assert_eq!(drawn("review-row-5001", &mut cx), None);

    // The wheel moves the rows and leaves the reviewer on their line.
    let on = position(pane, &mut cx);
    let over_the_rows = bounds_of_row(5, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    assert_eq!(in_view(&mut cx), 100..121);
    assert_eq!(drawn("review-row-20", &mut cx), None);
    assert!(drawn("review-row-100", &mut cx).is_some());
    assert_eq!(position(pane, &mut cx), on);
    assert_eq!(
        on,
        Position {
            path: path.clone(),
            side: Side::New,
            line: 0
        }
    );

    // The next key moves the reviewer one line on from where they are, not from where
    // the wheel went, and the view comes back to them.
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
    assert_eq!(row, Some(3));
    assert!(in_view(&mut cx).contains(&3));

    // Walking past the last row in view takes the view along.
    for _ in 0..40 {
        cx.simulate_keystrokes("j");
    }
    let row = pane
        .read_with(&cx, |pane, _| pane.position_row())
        .expect("a row");
    assert_eq!(row, 43);
    assert!(in_view(&mut cx).contains(&row));
    let drawn = bounds_of_row(row, &mut cx);
    assert!(drawn.top() >= px(0.) && drawn.bottom() <= WINDOW.height);
}
