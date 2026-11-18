//! Component test: the Review pane on GPUI's test platform, with no window on any screen.
//! Keys, clicks and the wheel go in; the pane is read back through the model it shows.
//! The test plays the Keeper on the other end of a socket pair: it answers what the
//! Window asks for the changes and for each file, and tells it facts.

mod common;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_diff::{Row, Side};
use yaar_frame::SetBase;
use yaar_link::{Changes, Comparison, Fact, FileStatus, NoComparison};
use yaar_review_pane::{PaneRow, Position, ReviewPane};

use common::{
    Stage, bounds_of_row, commit, default_base, drawn, file, id, open, rows, since_main, texts,
    wait_for_files, wait_until, workspace,
};

/// The rows of the pane as a reader of a unified diff would write them down: a heading
/// for each file, then each line the pane shows with the sign of its row, its text read
/// through the pane.
fn reading(pane: &gpui_kit::Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<String> {
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
            _ => Ok(yaar_link::FileSides {
                old: yaar_link::Content::Absent,
                new: yaar_link::Content::Text(b"alpha\nbeta\n".to_vec()),
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
