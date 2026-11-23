//! Component test: a worktree made with an agent in it, as the terminal pane carries out
//! what the Frame wants (`Wanted::Worktree`), on GPUI's test platform with no window on any
//! screen. The test plays the Keeper on the other end of a socket pair, as it plays it for
//! every terminal the pane opens: it hears the request, tells the list of workspaces that
//! holds the new worktree, and answers.
//!
//! The Keeper tells the list before it answers, and the Frame's model puts a workspace on
//! screen even when its reply comes first; both orders are played.

mod common;

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_frame::Wanted;
use yaar_link::{Fact, Program, Reply, Request, RequestId, TerminalId, TerminalInfo, WorkspaceId};
use yaar_terminal_pane::WORKTREE_BEING_MADE;

use common::{
    Panes, lines, open_panes, refused, terminal, two_workspaces, wait_drawn, wait_until,
    workspace_id,
};

fn wanted() -> Wanted {
    Wanted::Worktree {
        repository: common::one_repository().id,
        from: "main".to_owned(),
        branch: "feature-x".to_owned(),
        agent: "claude".to_owned(),
    }
}

/// Has the Frame want a worktree, and gives the Keeper's end of the request.
fn want(panes: &mut Panes, cx: &mut VisualTestContext) -> (RequestId, Request) {
    panes
        .models
        .terminals
        .update(cx, |list, cx| list.want(wanted(), cx));
    let (id, request) = panes.keeper.request(cx);
    (id, request)
}

fn notice(panes: &Panes, cx: &mut VisualTestContext) -> Option<String> {
    panes.pane.read_with(cx, |pane, _| pane.notice())
}

fn on_screen(panes: &Panes, cx: &mut VisualTestContext) -> Option<WorkspaceId> {
    panes
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
}

/// The terminals the shared list of the window holds.
fn listed(panes: &Panes, cx: &mut VisualTestContext) -> Vec<TerminalInfo> {
    panes
        .models
        .terminals
        .read_with(cx, |list, _| list.terminals().to_vec())
}

/// A window with a terminal open in the workspace on screen whose grid the painter fitted,
/// so that the size the pane lays terminals out at is not the one it starts with.
fn fitted(cx: &mut TestAppContext) -> (Panes, VisualTestContext, TerminalId) {
    let (mut panes, mut cx) = open_panes(cx);
    let first = terminal('a');
    panes.open_terminal(&first, &mut cx);
    let keeper = &panes.keeper;
    wait_until(&mut cx, "the terminal to be fitted to the room", |_| {
        !keeper.resizes(&first).is_empty()
    });
    (panes, cx, first)
}

#[gpui_kit::test]
fn a_worktree_wanted_is_asked_for_at_the_panes_size_and_its_agent_shows_in_the_new_workspace(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, first) = fitted(cx);
    let size = panes
        .keeper
        .resizes(&first)
        .last()
        .copied()
        .expect("a fitted size");

    let (id, request) = want(&mut panes, &mut cx);
    assert_eq!(
        request,
        Request::CreateWorktree {
            repository: common::one_repository().id,
            from: "main".to_owned(),
            branch: "feature-x".to_owned(),
            agent: "claude".to_owned(),
            size,
        }
    );
    // While the answer is awaited the notice says what is being made.
    assert_eq!(
        notice(&panes, &mut cx),
        Some(format!("{WORKTREE_BEING_MADE} feature-x"))
    );
    assert_eq!(on_screen(&panes, &mut cx), Some(workspace_id('1')));

    // The Keeper tells the list that holds the new workspace, then answers; what the agent
    // writes follows the answer at once.
    let second = terminal('b');
    panes.keeper.tell(Fact::Workspaces(vec![two_workspaces()]));
    panes.keeper.reply(
        id,
        Ok(Reply::WorktreeCreated {
            workspace: workspace_id('2'),
            terminal: second.clone(),
        }),
    );
    panes.keeper.output(&second, b"agent says hello");

    // The new workspace is on screen, with the agent's terminal shown in it and the keys
    // going to it.
    let pane = panes.pane.clone();
    wait_until(&mut cx, "the agent's terminal in the pane", |cx| {
        pane.read_with(cx, |pane, _| pane.terminals().contains(&second))
    });
    wait_drawn(
        &mut cx,
        "the new workspace on screen with the terminal focused",
        |cx| {
            on_screen(&panes, cx) == Some(workspace_id('2'))
                && panes.focused_terminal(cx).as_ref() == Some(&second)
        },
    );
    let shown = panes.pane.read_with(&cx, |pane, _| pane.shown());
    assert_eq!(shown, vec![second.clone()]);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&second)),
        Some(workspace_id('2'))
    );
    wait_until(&mut cx, "the agent's words", |cx| {
        lines(&pane, &second, cx).first().map(String::as_str) == Some("agent says hello")
    });
    assert_eq!(notice(&panes, &mut cx), None);
    // The shared list holds it as the agent's terminal, which no pane attaches to again.
    assert!(listed(&panes, &mut cx).contains(&TerminalInfo {
        id: second.clone(),
        workspace: workspace_id('2'),
        program: Program::Agent {
            agent: "claude".to_owned()
        },
    }));
    // What is typed now reaches the agent.
    cx.simulate_input("x");
    assert_eq!(panes.keeper.input(&mut cx), (second, b"x".to_vec()));
}

#[gpui_kit::test]
fn a_reply_that_comes_before_the_list_puts_the_new_workspace_on_screen_once_it_is_listed(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, _) = fitted(cx);
    // The user named the branch in a field of the Frame: the focus is not in the pane.
    panes.click_elsewhere(&mut cx);
    assert_eq!(panes.focused_terminal(&mut cx), None);
    let (id, _) = want(&mut panes, &mut cx);
    let second = terminal('b');
    panes.keeper.reply(
        id,
        Ok(Reply::WorktreeCreated {
            workspace: workspace_id('2'),
            terminal: second.clone(),
        }),
    );
    let pane = panes.pane.clone();
    wait_until(&mut cx, "the agent's terminal in the pane", |cx| {
        pane.read_with(cx, |pane, _| pane.terminals().contains(&second))
    });
    // The workspace is not listed yet: the one on screen stays.
    assert_eq!(on_screen(&panes, &mut cx), Some(workspace_id('1')));

    panes.keeper.tell(Fact::Workspaces(vec![two_workspaces()]));
    wait_drawn(
        &mut cx,
        "the new workspace on screen with the terminal focused",
        |cx| {
            on_screen(&panes, cx) == Some(workspace_id('2'))
                && panes.focused_terminal(cx).as_ref() == Some(&second)
        },
    );
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.shown()), [second]);
}

#[gpui_kit::test]
fn a_worktree_the_keeper_refuses_opens_nothing_and_the_reason_is_the_notice(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, first) = fitted(cx);
    let (id, _) = want(&mut panes, &mut cx);
    assert_eq!(
        notice(&panes, &mut cx),
        Some(format!("{WORKTREE_BEING_MADE} feature-x"))
    );

    panes
        .keeper
        .reply(id, Err(refused("the branch feature-x exists already")));
    wait_until(&mut cx, "the refusal", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.notice())
            .is_some_and(|notice| !notice.starts_with(WORKTREE_BEING_MADE))
    });
    assert_eq!(
        notice(&panes, &mut cx),
        Some("the branch feature-x exists already".to_owned())
    );
    // Nothing was opened or shown, and the workspace on screen is the same. A terminal
    // asked for after it, through the same path, is the barrier for what a refusal might
    // have set off late.
    panes.open_terminal(&terminal('c'), &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [first, terminal('c')]
    );
    assert_eq!(on_screen(&panes, &mut cx), Some(workspace_id('1')));
    assert_eq!(listed(&panes, &mut cx).len(), 2);
}
