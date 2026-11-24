//! Component test: the composed Window with every pane over the built Keeper, which holds real
//! repositories and starts real shells.
//! What the panes show is read through their models and the elements they draw.

mod common;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_link::{Review, TerminalId, Thread, WorkspaceId};

use common::{Composed, drawn, key_of, open_composed, wait_until};

fn on_screen(window: &Composed, cx: &mut VisualTestContext) -> WorkspaceId {
    window
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
        .expect("a workspace is on screen")
}

fn shown(window: &Composed, cx: &mut VisualTestContext) -> Vec<TerminalId> {
    window.terminals.read_with(cx, |pane, _| pane.shown())
}

fn thread_saying(
    window: &Composed,
    text: &str,
    cx: &mut VisualTestContext,
) -> Option<(Review, Thread)> {
    window.review.read_with(cx, |pane, _| {
        let review = pane.review()?;
        let thread = review
            .threads
            .iter()
            .find(|thread| thread.comments.first().is_some_and(|c| c.text == text))?;
        Some((review.clone(), thread.clone()))
    })
}

fn open_a_terminal(window: &Composed, cx: &mut VisualTestContext) -> TerminalId {
    let before = window.terminals.read_with(cx, |pane, _| pane.terminals());
    cx.simulate_keystrokes(&key_of("New Terminal"));
    wait_until(cx, "the new terminal in the terminal pane", |cx| {
        window
            .terminals
            .read_with(cx, |pane, _| pane.terminals().len() > before.len())
    });
    let opened = window
        .terminals
        .read_with(cx, |pane, _| pane.terminals())
        .into_iter()
        .find(|terminal| !before.contains(terminal))
        .expect("the pane holds a terminal that is new");
    wait_until(cx, "the terminal shown", |cx| {
        shown(window, cx) == [opened.clone()]
    });
    opened
}

fn comment(window: &Composed, text: &str, cx: &mut VisualTestContext) {
    window.click_into_the_review_pane(cx);
    cx.simulate_keystrokes("c");
    wait_until(cx, "the comment box", |cx| {
        window
            .review
            .read_with(cx, |pane, _| pane.comment_box().is_some())
    });
    cx.simulate_input(text);
    cx.simulate_keystrokes("escape");
    wait_until(cx, "the draft with its whole text", |cx| {
        thread_saying(window, text, cx).is_some()
    });
}

fn assert_shows_only(
    window: &Composed,
    workspace: &WorkspaceId,
    (mine, terminal): (&str, &TerminalId),
    (other, others_terminal): (&str, &TerminalId),
    cx: &mut VisualTestContext,
) {
    wait_until(cx, "the reviews of the workspace on screen", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.reviews()
                .is_some_and(|reviews| reviews.workspace == *workspace)
        }) && thread_saying(window, mine, cx).is_some()
    });
    assert!(
        thread_saying(window, other, cx).is_none(),
        "the review pane shows {other:?} of the other worktree"
    );
    let (_, thread) = thread_saying(window, mine, cx).expect("the thread of this worktree");
    let draft = thread.comments[0].id.clone();
    wait_until(cx, "the comment row of the thread", |cx| {
        drawn(&format!("review-comment-{draft}:draft"), cx).is_some()
    });

    assert_eq!(shown(window, cx), std::slice::from_ref(terminal));
    assert_eq!(
        window
            .terminals
            .read_with(cx, |pane, _| pane.workspace(terminal)),
        Some(workspace.clone())
    );
    assert_eq!(
        window.terminals.read_with(cx, |pane, _| pane.focused()),
        Some(terminal.clone()),
    );
    wait_until(cx, "the slot of the terminal", |cx| {
        cx.debug_bounds("terminal-slot-0").is_some()
    });
    assert!(
        cx.debug_bounds("terminal-slot-1").is_none(),
        "one terminal is on show, not two"
    );
    let (mine, others) = (terminal.as_str(), others_terminal.as_str());
    wait_until(cx, "the view of the terminal", |cx| {
        drawn(&format!("terminal-{mine}"), cx).is_some()
    });
    assert!(
        drawn(&format!("terminal-{others}"), cx).is_none(),
        "the view of the other worktree's terminal is drawn"
    );
    assert!(
        !shown(window, cx).contains(others_terminal),
        "the terminal of the other worktree is on show"
    );
}

#[gpui_kit::test]
fn selecting_a_workspace_shows_the_reviews_of_its_worktree_and_its_terminals_together(
    cx: &mut TestAppContext,
) {
    let (mut window, mut cx) = open_composed(cx, Vec::new());

    // A folder opened comes on screen, so each worktree gets its terminal and comment while it is
    // on screen.
    window.open_a_folder(&mut cx);
    let a = on_screen(&window, &mut cx);
    let a_terminal = open_a_terminal(&window, &mut cx);
    comment(&window, "on A", &mut cx);

    window.open_a_folder(&mut cx);
    let b = on_screen(&window, &mut cx);
    assert_ne!(a, b);
    let b_terminal = open_a_terminal(&window, &mut cx);
    comment(&window, "on B", &mut cx);
    assert_ne!(a_terminal, b_terminal);

    let row = window.point_in(&format!("sidebar-workspace-{a}"), &mut cx);
    cx.simulate_click(row, Modifiers::none());
    assert_eq!(on_screen(&window, &mut cx), a);
    assert_shows_only(
        &window,
        &a,
        ("on A", &a_terminal),
        ("on B", &b_terminal),
        &mut cx,
    );

    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes(&key_of("Next Workspace"));
    assert_eq!(on_screen(&window, &mut cx), b);
    assert_shows_only(
        &window,
        &b,
        ("on B", &b_terminal),
        ("on A", &a_terminal),
        &mut cx,
    );
}
