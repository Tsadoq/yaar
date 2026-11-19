//! Component test: the composed Window on GPUI's test platform, with every pane and the
//! built Keeper program beneath it, which holds real repositories and starts real shells.
//! No window is on any screen.
//!
//! Keys, clicks and typed text go in as a user's do; what the panes show is read back
//! through the models they draw from, and through the elements they draw. The test waits
//! for facts with a deadline and never for time.

mod common;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_link::{Review, TerminalId, Thread, WorkspaceId};

use common::{Composed, drawn, key_of, open_composed, wait_until};

/// The workspace on screen.
fn on_screen(window: &Composed, cx: &mut VisualTestContext) -> WorkspaceId {
    window
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
        .expect("a workspace is on screen")
}

/// The terminals the terminal pane draws, left to right.
fn shown(window: &Composed, cx: &mut VisualTestContext) -> Vec<TerminalId> {
    window.terminals.read_with(cx, |pane, _| pane.shown())
}

/// The thread of the review on screen whose first comment says `text`, if the review pane
/// shows one, with the review.
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

/// Opens a terminal for the workspace on screen by the key of New Terminal and waits
/// until the terminal pane shows it. Returns it.
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

/// Writes a comment on the line the reviewer is on, as a user does: into the review pane,
/// `c`, the text, Escape; and waits until the pane holds the draft with its whole text.
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

/// Holds both panes to the workspace `workspace`, the thread that says `mine` and the
/// terminal `terminal`, and to nothing of `other` and `others_terminal`.
fn assert_shows_only(
    window: &Composed,
    workspace: &WorkspaceId,
    (mine, terminal): (&str, &TerminalId),
    (other, others_terminal): (&str, &TerminalId),
    cx: &mut VisualTestContext,
) {
    // The review pane: the reviews of that workspace, with its comment.
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

    // The terminal pane: its terminals, and not the other's.
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

    // Two worktrees, each with a terminal of its own and a comment on its change. A
    // folder opened comes on screen, so each is worked on while it is.
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

    // Selecting a workspace: one click on its row in the sidebar.
    let row = window.point_in(&format!("sidebar-workspace-{a}"), &mut cx);
    cx.simulate_click(row, Modifiers::none());
    // Both panes are read after that click and no other input: the one thing a user did
    // is to select, and everything of the worktree shows.
    assert_eq!(on_screen(&window, &mut cx), a);
    assert_shows_only(
        &window,
        &a,
        ("on A", &a_terminal),
        ("on B", &b_terminal),
        &mut cx,
    );

    // The key of Next Workspace, pressed where it works, puts the other on screen, and
    // everything of that one shows.
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
