//! Component test: the terminal pane with two workspaces, on GPUI's test platform with no
//! window on any screen. The workspace on screen is chosen by a click on its row in the
//! sidebar or by the key of Next Workspace; the pane is read back through the model it
//! shows. The test plays the Keeper on the other end of a socket pair.

mod common;

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_link::{Reply, Request, TerminalId, WorkspaceId};
use yaar_terminal_pane::{NewTerminal, NextTerminal, PreviousTerminal, ToggleSplit};

use common::{
    Panes, click_in, drawn, key_of, open_panes_listing, terminal, two_workspaces, wait_until,
    workspace_id,
};

fn open_two(cx: &mut TestAppContext) -> (Panes, VisualTestContext) {
    open_panes_listing(cx, &[two_workspaces()], |_, _| {})
}

/// Puts a workspace on screen by a click on its row in the sidebar.
fn select(workspace: &WorkspaceId, panes: &Panes, cx: &mut VisualTestContext) {
    click_in(&format!("sidebar-workspace-{workspace}"), cx);
    let on_screen = panes
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned());
    assert_eq!(on_screen.as_ref(), Some(workspace));
}

fn shown(panes: &Panes, cx: &mut VisualTestContext) -> Vec<TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.shown())
}

fn focused(panes: &Panes, cx: &mut VisualTestContext) -> Option<TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.focused())
}

fn terminals(panes: &Panes, cx: &mut VisualTestContext) -> Vec<TerminalId> {
    panes.pane.read_with(cx, |pane, _| pane.terminals())
}

fn split(panes: &Panes, cx: &mut VisualTestContext) -> bool {
    panes.pane.read_with(cx, |pane, _| pane.split())
}

/// Whether the focus is in the view of this terminal.
fn has_the_focus(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> bool {
    let view = panes
        .pane
        .read_with(cx, |pane, _| pane.terminal(id))
        .expect("a terminal of the pane");
    let focus = view.read_with(cx, |view, _| view.focus_handle());
    cx.update(|window, _| focus.is_focused(window))
}

#[gpui_kit::test]
fn a_workspace_on_screen_shows_its_own_terminals_and_gets_them_back_when_it_returns(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2) = (terminal('a'), terminal('b'));

    panes.open_terminal(&t1, &mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t1));

    // The other workspace has none: the pane says how to open one, shows no terminal, and
    // keeps the one of the first alive in its list.
    select(&two, &panes, &mut cx);
    assert_eq!(shown(&panes, &mut cx), []);
    assert_eq!(focused(&panes, &mut cx), None);
    assert!(drawn("terminal-slot-0", &mut cx).is_none());
    assert!(drawn("terminal-how-to-open", &mut cx).is_some());
    assert_eq!(terminals(&panes, &mut cx), std::slice::from_ref(&t1));

    // A terminal opened there is shown, and is of that workspace.
    panes.open_terminal(&t2, &mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t2));
    assert!(drawn("terminal-how-to-open", &mut cx).is_none());
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t2)),
        Some(two.clone())
    );

    // Back: the first terminal is shown again with the focus, and the second is not.
    select(&one, &panes, &mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t1));
    assert_eq!(focused(&panes, &mut cx), Some(t1.clone()));
    assert!(drawn("terminal-slot-1", &mut cx).is_none());
    // Every terminal the window opened is listed, in the order they were opened.
    assert_eq!(terminals(&panes, &mut cx), [t1.clone(), t2.clone()]);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t1)),
        Some(one)
    );
}

#[gpui_kit::test]
fn each_workspace_keeps_which_terminal_had_the_focus_and_whether_it_was_split(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2, t3, t4) = (terminal('a'), terminal('b'), terminal('c'), terminal('d'));

    // Two terminals in each workspace, opened in turn.
    panes.open_terminal(&t1, &mut cx);
    select(&two, &panes, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    select(&one, &panes, &mut cx);
    panes.open_terminal(&t3, &mut cx);
    select(&two, &panes, &mut cx);
    panes.open_terminal(&t4, &mut cx);

    // The first workspace: split, with the second slot's terminal focused. The second
    // workspace is not split and has its first terminal on show.
    select(&one, &panes, &mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t3));
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), [t3.clone(), t1.clone()]);
    panes.click_slot(1, &mut cx);
    assert_eq!(focused(&panes, &mut cx), Some(t1.clone()));

    select(&two, &panes, &mut cx);
    assert!(!split(&panes, &mut cx));
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t4));
    panes.press_in_the_app::<PreviousTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t2));

    // Each comes back as it was left, and the other's terminals are in neither slot.
    select(&one, &panes, &mut cx);
    assert!(split(&panes, &mut cx));
    assert_eq!(shown(&panes, &mut cx), [t3.clone(), t1.clone()]);
    assert_eq!(focused(&panes, &mut cx), Some(t1.clone()));
    assert!(drawn("terminal-slot-1", &mut cx).is_some());

    select(&two, &panes, &mut cx);
    assert!(!split(&panes, &mut cx));
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t2));
    assert_eq!(focused(&panes, &mut cx), Some(t2));
    assert!(drawn("terminal-slot-1", &mut cx).is_none());
}

#[gpui_kit::test]
fn next_previous_and_split_go_among_the_terminals_of_the_workspace_on_screen(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2, t3) = (terminal('a'), terminal('b'), terminal('c'));

    // Opened in the order t1 (one), t2 (two), t3 (one): the list holds the other
    // workspace's between them.
    panes.open_terminal(&t1, &mut cx);
    select(&two, &panes, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    select(&one, &panes, &mut cx);
    panes.open_terminal(&t3, &mut cx);
    assert_eq!(
        terminals(&panes, &mut cx),
        [t1.clone(), t2.clone(), t3.clone()]
    );

    // From the last of this workspace round to its first, never to t2.
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t3));
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t1));
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t3));
    panes.press_in_the_app::<PreviousTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t1));
    panes.press_in_the_app::<PreviousTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t3));

    // The split shows two of this workspace's, not the one after in the list.
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), [t3.clone(), t1.clone()]);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t3));

    // The other workspace has one: three terminals are open and there is nothing to split
    // or to step to.
    select(&two, &panes, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t2));
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t2));
    panes.press_in_the_app::<PreviousTerminal>(&mut cx);
    assert_eq!(shown(&panes, &mut cx), [t2]);
}

#[gpui_kit::test]
fn a_workspace_with_no_terminal_on_screen_keeps_the_focus_on_the_pane_and_keys_do_nothing(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let t1 = terminal('a');
    panes.open_terminal(&t1, &mut cx);
    select(&workspace_id('2'), &panes, &mut cx);

    // The Frame gives the focus to the pane, which has no terminal to hand it on to.
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    assert!(cx.update(|window, _| panes.pane_focus.is_focused(window)));
    assert!(!has_the_focus(&panes, &t1, &mut cx));

    for key in [
        key_of::<NextTerminal>(),
        key_of::<PreviousTerminal>(),
        key_of::<ToggleSplit>(),
    ] {
        cx.simulate_keystrokes(key);
        assert_eq!(shown(&panes, &mut cx), [], "after {key}");
        assert_eq!(focused(&panes, &mut cx), None);
    }
    // The terminal of the first workspace took no key.
    assert!(!has_the_focus(&panes, &t1, &mut cx));

    // Back there, the focus given to the pane goes on to its terminal.
    select(&workspace_id('1'), &panes, &mut cx);
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    assert!(has_the_focus(&panes, &t1, &mut cx));
}

#[gpui_kit::test]
fn a_terminal_whose_answer_comes_after_another_workspace_came_on_screen_is_not_shown_there(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (late, t2) = (terminal('a'), terminal('b'));
    select(&two, &panes, &mut cx);
    panes.open_terminal(&t2, &mut cx);

    // A terminal is asked for in the first workspace, and before the answer the user puts
    // the second on screen.
    select(&one, &panes, &mut cx);
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(
        request,
        Request::OpenTerminal { ref workspace, .. } if *workspace == one
    ));
    select(&two, &panes, &mut cx);
    panes.keeper.reply(
        id,
        Ok(Reply::TerminalOpened {
            terminal: late.clone(),
        }),
    );
    wait_until(&mut cx, "the terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&late))
    });

    // The second workspace on screen shows what it showed, and the focus stays where it
    // is. The terminal belongs to the first, which shows it when it returns.
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t2));
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&late)),
        Some(one.clone())
    );
    assert!(!has_the_focus(&panes, &late, &mut cx));
    select(&one, &panes, &mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&late));
    assert_eq!(focused(&panes, &mut cx), Some(late));
}

#[gpui_kit::test]
fn a_click_on_another_workspace_with_the_focus_in_a_terminal_moves_the_focus_to_that_workspaces_terminal(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    select(&two, &panes, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    assert!(has_the_focus(&panes, &t2, &mut cx));

    // The click on the sidebar's row leaves the focus in the terminal that is drawn
    // until the workspace changes. Then the other workspace's terminal has it, and the
    // terminal that is no longer drawn takes no key.
    select(&one, &panes, &mut cx);
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t1));
    assert!(has_the_focus(&panes, &t1, &mut cx));
    assert!(!has_the_focus(&panes, &t2, &mut cx));
}

#[gpui_kit::test]
fn a_click_on_a_workspace_with_no_terminal_with_the_focus_in_a_terminal_moves_the_focus_to_the_pane(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_two(cx);
    let t1 = terminal('a');
    panes.open_terminal(&t1, &mut cx);
    assert!(has_the_focus(&panes, &t1, &mut cx));

    // The other workspace has no terminal to hand the focus on to: the pane itself has
    // it, and the terminal of the first, no longer drawn, does not.
    select(&workspace_id('2'), &panes, &mut cx);
    assert_eq!(shown(&panes, &mut cx), []);
    assert!(cx.update(|window, _| panes.pane_focus.is_focused(window)));
    assert!(!has_the_focus(&panes, &t1, &mut cx));

    // Back, with the focus in the pane: it goes on to the terminal that is shown again.
    select(&workspace_id('1'), &panes, &mut cx);
    assert!(has_the_focus(&panes, &t1, &mut cx));
}

// A key of the app works while a terminal has the focus on macOS only: there the focus is
// in the terminal when the key changes the workspace, and has to follow it.
#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn the_key_for_the_next_workspace_with_the_focus_in_a_terminal_moves_the_focus_with_it(
    cx: &mut TestAppContext,
) {
    use yaar_frame::SelectNextWorkspace;

    let (mut panes, mut cx) = open_two(cx);
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    select(&workspace_id('2'), &panes, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    assert!(has_the_focus(&panes, &t2, &mut cx));

    cx.simulate_keystrokes(key_of::<SelectNextWorkspace>());
    assert_eq!(shown(&panes, &mut cx), std::slice::from_ref(&t1));
    assert!(has_the_focus(&panes, &t1, &mut cx));
    assert!(!has_the_focus(&panes, &t2, &mut cx));
}
