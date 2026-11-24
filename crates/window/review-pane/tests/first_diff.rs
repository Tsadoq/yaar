//! Component test: the mark of the first diff, on GPUI's test platform. The pane marks
//! `first_diff` on the frame after the one that first draws rows with a line of a diff, in
//! Changes, once for the process. GPUI's test platform has no frame loop, so a test delivers
//! the next frame itself, as `Window::simulate_next_frame` does. A test reads whether the
//! mark was taken through `yaar_frame::trace::first_of`, which asks the same question the
//! mark does and answers true only to the first asker. Each test is in a function of its own
//! because the mark is the process's: nextest runs each in a process of its own, and
//! `cargo test` of this binary fails by design. A test never sets `YAAR_TRACE_START`, so no
//! mark prints.

mod common;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_link::FileStatus;
use yaar_review_pane::View;

use common::{commit, drawn, file, open, since_main, texts, wait_for_files, workspace};

/// Delivers the next frame, which the test platform never does by itself, and says how many
/// callbacks asked for it were run.
fn deliver_next_frame(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| window.simulate_next_frame(cx))
}

/// Draws the window again, lets what the draw started finish, and delivers the frame. It
/// says how many callbacks asked for that frame were run.
fn draw_again(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    deliver_next_frame(cx)
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
    // The frame after the draw is delivered; a second draw does not ask for the mark again.
    assert!(
        deliver_next_frame(&mut cx) > 0,
        "the pane asked for no callback on the next frame"
    );
    assert_eq!(
        draw_again(&mut cx),
        0,
        "the pane asked for the mark of the first diff again on a later draw"
    );

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

#[gpui_kit::test]
fn a_pane_that_drew_a_diff_marks_it_on_the_next_frame_and_not_before(cx: &mut TestAppContext) {
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
    // The pane holds a line of a diff and was drawn with it, and no frame came after.
    assert!(stage.pane.read_with(&cx, |pane, _| {
        pane.rows()
            .iter()
            .any(|row| matches!(row, yaar_review_pane::PaneRow::Diff { .. }))
    }));

    assert!(
        yaar_frame::trace::first_of("first_diff"),
        "the pane took the mark of the first diff before the frame that shows it came"
    );
    assert!(
        deliver_next_frame(&mut cx) > 0,
        "the pane asked for no callback on the next frame"
    );
}

#[gpui_kit::test]
fn a_pane_in_flow_that_held_a_diff_did_not_take_the_mark_of_the_first_diff(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    // Flow takes the place of Changes, and the pane has the focus for the key that shows it.
    let bar = drawn("review-bar", &mut cx).expect("the bar is drawn");
    cx.simulate_click(bar.center(), Modifiers::none());
    cx.simulate_keystrokes("f");
    cx.run_until_parked();
    assert_eq!(stage.pane.read_with(&cx, |pane, _| pane.view()), View::Flow);
    let changes = since_main(
        vec![commit('c', "Count to five")],
        vec![file("src/numbers.txt", FileStatus::Modified)],
    );
    stage
        .keeper
        .show(&changes, |_| texts("one\ntwo\n", "one\n2\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    cx.run_until_parked();
    draw_again(&mut cx);
    draw_again(&mut cx);
    // The pane holds a line of a diff, which it did not draw: Flow is on screen.
    assert_eq!(stage.pane.read_with(&cx, |pane, _| pane.view()), View::Flow);
    assert!(stage.pane.read_with(&cx, |pane, _| {
        pane.rows()
            .iter()
            .any(|row| matches!(row, yaar_review_pane::PaneRow::Diff { .. }))
    }));

    assert!(
        yaar_frame::trace::first_of("first_diff"),
        "the pane took the mark of the first diff while Flow was on screen"
    );
}
