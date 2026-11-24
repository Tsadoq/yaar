//! Component test: the mark of the first diff, on GPUI's test platform. The pane marks
//! `first_diff` on the frame after it first draws a diff with rows, once for the process; a
//! test reads whether the mark was taken through `yaar_frame::trace::first_of`, which asks
//! the same question the mark does and answers true only to the first asker. Each test is in
//! a function of its own because the mark is the process's: nextest runs each in a process
//! of its own. A test never sets `YAAR_TRACE_START`, so no mark prints.

mod common;

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_link::FileStatus;

use common::{commit, file, open, since_main, texts, wait_for_files, workspace};

/// Draws the window again, and lets what the draw started finish.
fn draw_again(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_pane_that_drew_a_diff_took_the_mark_of_the_first_diff(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        vec![commit('c', "Count to five")],
        vec![file("src/numbers.txt", FileStatus::Modified)],
    );
    stage
        .keeper
        .show(&changes, |_| texts("one\ntwo\n", "one\n2\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    cx.run_until_parked();
    // A second draw does not take the mark again, and the mark does not start over.
    draw_again(&mut cx);

    assert!(
        !yaar_frame::trace::first_of("first_diff"),
        "the pane drew a diff and did not mark the first diff"
    );
}

#[gpui_kit::test]
fn a_pane_that_drew_no_change_did_not_take_the_mark_of_the_first_diff(cx: &mut TestAppContext) {
    let (stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    cx.run_until_parked();
    draw_again(&mut cx);
    assert!(stage.pane.read_with(&cx, |pane, _| pane.rows().is_empty()));

    assert!(
        yaar_frame::trace::first_of("first_diff"),
        "the pane drew no diff and marked the first diff"
    );
}

#[gpui_kit::test]
fn a_pane_that_drew_commits_and_no_diff_did_not_take_the_mark_of_the_first_diff(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(vec![commit('c', "Count to five")], vec![]);
    stage
        .keeper
        .show(&changes, |_| unreachable!("no file"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    draw_again(&mut cx);
    // The pane shows rows, and none of them is a line of a diff.
    assert!(stage.pane.read_with(&cx, |pane, _| {
        !pane.rows().is_empty()
            && !pane
                .rows()
                .iter()
                .any(|row| matches!(row, yaar_review_pane::PaneRow::Diff { .. }))
    }));

    assert!(
        yaar_frame::trace::first_of("first_diff"),
        "the pane drew no line of a diff and marked the first diff"
    );
}
