//! Component test: the terminal pane with the agents the Keeper tells of, on GPUI's test
//! platform with no window on any screen. The test plays the Keeper on the other end of a
//! socket pair: it tells which sessions run in which terminals and what they do, and the
//! pane is read back through the model it shows and the elements it draws.
//!
//! A negative clause is read after a barrier: something the test causes after it, through
//! the same path, whose arrival shows that everything before it has arrived.

mod common;

use std::time::{Duration, Instant};

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_link::{Fact, Session, SessionId, SessionState, TerminalId, WorkspaceAgents, WorkspaceId};
use yaar_terminal_pane::ToggleSplit;

use common::{
    DEADLINE, Panes, click_in, drawn, lines, open_panes, open_panes_listing, terminal,
    two_workspaces, wait_until, workspace_id,
};

/// Keeps the app running until `holds` says yes: [`wait_until`] for what is read from the
/// window as drawn. Fails at the deadline, naming `what`.
fn wait_drawn(
    cx: &mut VisualTestContext,
    what: &str,
    mut holds: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain for {what}");
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// A session of Claude Code in `workspace`, in `terminal` when there is one.
fn session(
    name: char,
    workspace: &WorkspaceId,
    terminal: Option<&TerminalId>,
    state: SessionState,
) -> Session {
    Session {
        id: SessionId::new(name.to_string().repeat(32)),
        agent: "claude".to_owned(),
        title: format!("Claude Code {name}"),
        workspace: workspace.clone(),
        terminal: terminal.cloned(),
        state,
        turn: None,
        review: None,
        attached: None,
        resumable: false,
        turn_from_user: false,
    }
}

/// Tells the Window the sessions of one workspace, as the Keeper does when they change.
fn tell_agents(panes: &Panes, workspace: &WorkspaceId, sessions: Vec<Session>) {
    let shown = sessions.iter().map(|session| session.state).min();
    panes.keeper.tell(Fact::AgentsChanged(WorkspaceAgents {
        workspace: workspace.clone(),
        shown,
        sessions,
    }));
}

fn waiting(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> bool {
    panes.pane.read_with(cx, |pane, cx| pane.waiting(id, cx))
}

/// Waits until the Window holds the agents of `workspace`: what was told before is read.
fn barrier(panes: &Panes, workspace: &WorkspaceId, cx: &mut VisualTestContext) {
    let (models, workspace) = (panes.models.clone(), workspace.clone());
    wait_until(cx, "the agents of the workspace", move |cx| {
        models
            .agents
            .read_with(cx, |agents, _| agents.of(&workspace).is_some())
    });
}

#[gpui_kit::test]
fn the_terminal_of_an_agent_that_waits_at_a_permission_prompt_carries_the_waiting_mark(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = workspace_id('1');
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    assert!(!waiting(&panes, &t1, &mut cx));
    assert!(drawn("terminal-waiting-0", &mut cx).is_none());

    // The agent in the terminal waits for permission: the terminal carries the mark.
    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            Some(&t1),
            SessionState::WaitsForPermission,
        )],
    );
    wait_drawn(&mut cx, "the waiting mark", |cx| {
        drawn("terminal-waiting-0", cx).is_some()
    });
    assert!(waiting(&panes, &t1, &mut cx));

    // A second terminal, with no session, carries none; the first is in the second slot
    // of the split view and its mark is drawn there.
    panes.open_terminal(&t2, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        [t2.clone(), t1.clone()]
    );
    wait_drawn(&mut cx, "the mark in the second slot", |cx| {
        drawn("terminal-waiting-1", cx).is_some()
    });
    assert!(!waiting(&panes, &t2, &mut cx));
    assert!(drawn("terminal-waiting-0", &mut cx).is_none());

    // The state is works again: the mark goes.
    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, Some(&t1), SessionState::Works)],
    );
    wait_drawn(&mut cx, "the mark gone", |cx| {
        drawn("terminal-waiting-1", cx).is_none()
    });
    assert!(!waiting(&panes, &t1, &mut cx));
    assert!(!waiting(&panes, &t2, &mut cx));

    // Nothing was sent to the program for it: a key typed into the terminal is the first
    // input the Keeper receives, and it is the only one. What the pane drew is not on the
    // program's screen either.
    panes.click_slot(1, &mut cx);
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t1.clone(), b"k".to_vec()));
    assert_eq!(panes.keeper.inputs(&t1), [b"k".to_vec()]);
    assert!(panes.keeper.inputs(&t2).is_empty());
    for terminal in [&t1, &t2] {
        assert!(
            lines(&panes.pane, terminal, &cx)
                .iter()
                .all(String::is_empty)
        );
    }
}

#[gpui_kit::test]
fn a_session_that_reports_from_no_terminal_marks_none(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = workspace_id('1');
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);

    // The session of an agent that runs outside the app's terminals waits: no terminal
    // is marked. The agents of another workspace, told after, are the barrier.
    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, None, SessionState::WaitsForPermission)],
    );
    tell_agents(
        &panes,
        &workspace_id('2'),
        vec![session(
            'y',
            &workspace_id('2'),
            Some(&t2),
            SessionState::Works,
        )],
    );
    barrier(&panes, &workspace_id('2'), &mut cx);
    assert!(!waiting(&panes, &t1, &mut cx));
    assert!(drawn("terminal-waiting-0", &mut cx).is_none());
}

#[gpui_kit::test]
fn a_terminal_of_a_workspace_off_screen_carries_the_mark_when_it_comes_back(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let t1 = terminal('a');
    panes.open_terminal(&t1, &mut cx);

    // The workspace of the terminal is not on screen when its agent starts to wait.
    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    assert!(drawn("terminal-slot-0", &mut cx).is_none());
    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            Some(&t1),
            SessionState::WaitsForPermission,
        )],
    );
    barrier(&panes, &one, &mut cx);
    assert!(waiting(&panes, &t1, &mut cx));
    assert!(drawn("terminal-waiting-0", &mut cx).is_none());

    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    wait_drawn(&mut cx, "the mark of the terminal back on screen", |cx| {
        drawn("terminal-waiting-0", cx).is_some()
    });
}
