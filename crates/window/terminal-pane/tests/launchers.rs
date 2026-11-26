//! A negative clause is read after a barrier: the next request the Keeper hears, caused through the
//! same path after the key under test, shows that the key sent nothing.

mod common;

use gpui_kit::{
    Bounds, Modifiers, Pixels, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point,
    px,
};
use yaar_link::{
    AgentCli, Ending, Fact, Program, Reply, Repository, Request, Session, SessionId, SessionState,
    TerminalId, TerminalPlace, WorkspaceAgents, WorkspaceId,
};
use yaar_terminal_pane::{CloseTerminal, NewAgentTerminal, NewTerminal, NextAgentCli, ResumeAgent};

use common::{
    Panes, click_in, drawn, lines, open_panes_listing, open_panes_staged, refused, terminal,
    two_workspaces, wait_drawn, wait_until, workspace_id,
};

fn claude() -> AgentCli {
    AgentCli {
        agent: "claude".to_owned(),
        title: "Claude Code".to_owned(),
    }
}

/// Titles differ from the agent names, and in length the other way round for the two CLIs, so
/// the widths of the launchers tell the title from the name.
fn pi() -> AgentCli {
    AgentCli {
        agent: "pi".to_owned(),
        title: "Pi, the coding agent".to_owned(),
    }
}

fn agent(name: &str) -> Program {
    Program::Agent {
        agent: name.to_owned(),
    }
}

/// The Window asks for the CLIs when it connects, so they are set before.
fn open_naming(cx: &mut TestAppContext, clis: Vec<AgentCli>) -> (Panes, VisualTestContext) {
    open_naming_in(cx, common::one_repository(), clis)
}

fn open_naming_in(
    cx: &mut TestAppContext,
    repository: Repository,
    clis: Vec<AgentCli>,
) -> (Panes, VisualTestContext) {
    open_panes_staged(
        cx,
        &[repository],
        move |keeper| keeper.set_clis(clis),
        |_, _| {},
    )
}

fn wait_for_launchers(panes: &Panes, count: usize, cx: &mut VisualTestContext) {
    let pane = panes.pane.clone();
    wait_until(cx, "the launchers", move |cx| {
        pane.read_with(cx, |pane, cx| pane.launchers(cx).len() == count)
    });
}

fn bounds_of(selector: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    drawn(selector, cx).unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn chosen(panes: &Panes, cx: &mut VisualTestContext) -> Option<AgentCli> {
    panes.pane.read_with(cx, |pane, cx| pane.chosen_cli(cx))
}

fn open_launcher_menu(cx: &mut VisualTestContext) {
    wait_drawn(cx, "the launcher menu trigger", |cx| {
        drawn("terminal-launcher-menu", cx).is_some()
    });
    if drawn("terminal-launcher-popover", cx).is_none() {
        click_in("terminal-launcher-menu", cx);
        wait_drawn(cx, "the launcher menu", |cx| {
            drawn("terminal-launcher-popover", cx).is_some()
        });
    }
}

fn open_resume_menu(cx: &mut VisualTestContext) {
    wait_drawn(cx, "the resume trigger", |cx| {
        drawn("terminal-resume", cx).is_some()
    });
    if drawn("terminal-resume-popover", cx).is_none() {
        click_in("terminal-resume", cx);
        wait_drawn(cx, "the resume menu", |cx| {
            drawn("terminal-resume-popover", cx).is_some()
        });
    }
}

fn open_dock_menu(cx: &mut VisualTestContext) {
    wait_drawn(cx, "the dock trigger", |cx| {
        drawn("terminal-place", cx).is_some()
    });
    if drawn("terminal-place-popover", cx).is_none() {
        click_in("terminal-place", cx);
        wait_drawn(cx, "the dock menu", |cx| {
            drawn("terminal-place-popover", cx).is_some()
        });
    }
}

fn wait_for_chosen_mark(all: &[&str], agent: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    open_launcher_menu(cx);
    let others: Vec<String> = all
        .iter()
        .filter(|other| **other != agent)
        .map(|other| format!("terminal-launcher-chosen-{other}"))
        .collect();
    wait_drawn(cx, "the mark of the chosen launcher", |cx| {
        drawn(&format!("terminal-launcher-chosen-{agent}"), cx).is_some()
            && others.iter().all(|other| drawn(other, cx).is_none())
    });
    bounds_of(&format!("terminal-launcher-chosen-{agent}"), cx)
}

fn within(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.left() >= outer.left()
        && inner.right() <= outer.right()
        && inner.top() >= outer.top()
        && inner.bottom() <= outer.bottom()
}

#[gpui_kit::test]
fn next_to_the_shell_the_pane_offers_one_launcher_for_each_agent_cli_the_keeper_names(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    let one = workspace_id('1');
    wait_for_launchers(&panes, 2, &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, cx| pane.launchers(cx)),
        [claude(), pi()]
    );

    wait_drawn(&mut cx, "the terminal header bar", |cx| {
        drawn("terminal-launchers", cx).is_some()
            && drawn("terminal-launcher-add", cx).is_some()
            && drawn("terminal-launcher-menu", cx).is_some()
    });
    let row = bounds_of("terminal-launchers", &mut cx);
    assert_eq!(row.size.height, px(40.));
    for control in [
        bounds_of("terminal-launcher-add", &mut cx),
        bounds_of("terminal-launcher-menu", &mut cx),
        bounds_of("terminal-split-toggle", &mut cx),
        bounds_of("terminal-place", &mut cx),
    ] {
        assert!(within(control, row), "{control:?} lies outside {row:?}");
    }
    let how_to_open = bounds_of("terminal-how-to-open", &mut cx);
    assert!(row.bottom() <= how_to_open.top());
    assert_eq!(row.top(), bounds_of("elsewhere-pane", &mut cx).top());

    open_launcher_menu(&mut cx);
    let menu = bounds_of("terminal-launcher-popover", &mut cx);
    let (first, second, shell) = (
        bounds_of("terminal-launcher-claude", &mut cx),
        bounds_of("terminal-launcher-pi", &mut cx),
        bounds_of("terminal-launcher-shell", &mut cx),
    );
    assert!(first.bottom() <= second.top() && second.bottom() <= shell.top());
    let first_title = bounds_of("terminal-launcher-title-claude", &mut cx);
    let second_title = bounds_of("terminal-launcher-title-pi", &mut cx);
    assert!(
        second_title.size.width > first_title.size.width,
        "{second_title:?} is not wider than {first_title:?}: the launcher does not carry the CLI's title"
    );
    for launcher in [first, second, shell] {
        assert!(within(launcher, menu), "{launcher:?} lies outside {menu:?}");
    }

    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    let mark = wait_for_chosen_mark(&["claude", "pi"], "claude", &mut cx);
    assert!(within(mark, first));
    assert!(drawn("terminal-launcher-chosen-shell", &mut cx).is_none());

    // Close menu before checking terminal slot sizing.
    click_in("terminal-launcher-menu", &mut cx);
    wait_drawn(&mut cx, "launcher menu closed", |cx| {
        drawn("terminal-launcher-popover", cx).is_none()
    });

    let first_terminal = terminal('a');
    panes.open_terminal(&first_terminal, &mut cx);
    wait_drawn(&mut cx, "the terminal's slot", |cx| {
        drawn("terminal-slot-0", cx).is_some()
    });
    assert!(row.bottom() <= bounds_of("terminal-slot-0", &mut cx).top());
    wait_until(&mut cx, "the first fit", |_| {
        panes.keeper.resizes(&first_terminal).len() == 1
    });
    let size = panes.grid(&first_terminal);

    // Primary `+` button launches the chosen default CLI.
    click_in("terminal-launcher-add", &mut cx);
    panes.answer_open(&terminal('x'), agent("claude"), &mut cx);

    open_launcher_menu(&mut cx);
    click_in("terminal-launcher-pi", &mut cx);
    let (workspace, asked) = panes.answer_open(&terminal('b'), agent("pi"), &mut cx);
    assert_eq!((workspace, asked), (one.clone(), size));
    wait_drawn(&mut cx, "the focus in the new terminal", |cx| {
        panes.focused_terminal(cx) == Some(terminal('b'))
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [first_terminal.clone(), terminal('x'), terminal('b')]
    );
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        [terminal('b')]
    );
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));

    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('c'), agent("claude"), &mut cx);

    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(pi()));
    let mark = wait_for_chosen_mark(&["claude", "pi"], "pi", &mut cx);
    assert!(within(mark, bounds_of("terminal-launcher-pi", &mut cx)));
    click_in("terminal-launcher-menu", &mut cx);
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('d'), agent("pi"), &mut cx);

    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    wait_for_chosen_mark(&["claude", "pi"], "claude", &mut cx);
    click_in("terminal-launcher-menu", &mut cx);
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(pi()));
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('e'), agent("claude"), &mut cx);

    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        assert_eq!(panes.focused_terminal(&mut cx), Some(terminal('e')));
        cx.simulate_keystrokes(common::key_of::<NewAgentTerminal>());
        panes.answer_open(&terminal('f'), agent("claude"), &mut cx);
        panes.click_slot(0, &mut cx);
        cx.simulate_keystrokes(common::key_of::<NextAgentCli>());
        assert_eq!(chosen(&panes, &mut cx), Some(pi()));
        cx.simulate_keystrokes(common::key_of::<NewAgentTerminal>());
        panes.answer_open(&terminal('g'), agent("pi"), &mut cx);
    }
    assert!(panes.keeper.no_input_waits());
}

#[gpui_kit::test]
fn the_chosen_launcher_is_the_choice_of_the_frames_model_which_another_part_may_move(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    wait_for_launchers(&panes, 2, &mut cx);
    let model_choice = |panes: &Panes, cx: &mut VisualTestContext| {
        panes
            .models
            .agents
            .read_with(cx, |agents, _| agents.chosen_cli().cloned())
    };
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    wait_for_chosen_mark(&["claude", "pi"], "claude", &mut cx);

    panes
        .models
        .agents
        .update(&mut cx, |agents, cx| agents.choose_next_cli(cx));
    wait_for_chosen_mark(&["claude", "pi"], "pi", &mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(pi()));
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('a'), agent("pi"), &mut cx);

    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(model_choice(&panes, &mut cx), Some(claude()));
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    wait_for_chosen_mark(&["claude", "pi"], "claude", &mut cx);

    // Switching the default CLI from the launcher menu updates the shared model.
    click_in("terminal-launcher-default-pi", &mut cx);
    wait_for_chosen_mark(&["claude", "pi"], "pi", &mut cx);
    assert_eq!(model_choice(&panes, &mut cx), Some(pi()));
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('b'), agent("pi"), &mut cx);
}

#[gpui_kit::test]
fn with_no_cli_named_the_pane_offers_only_the_shell_and_the_key_says_so(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_naming_in(cx, two_workspaces(), Vec::new());
    open_launcher_menu(&mut cx);
    wait_drawn(&mut cx, "the shell's launcher", |cx| {
        drawn("terminal-launcher-shell", cx).is_some()
    });
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, cx| pane.launchers(cx).is_empty())
    );
    assert_eq!(chosen(&panes, &mut cx), None);
    assert!(drawn("terminal-launcher-claude", &mut cx).is_none());
    assert!(drawn("terminal-launcher-pi", &mut cx).is_none());
    click_in("terminal-launcher-menu", &mut cx);

    // A key that fell back to a shell would ask for one in the workspace on screen, so the barrier
    // is a shell asked for in the other workspace.
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("The Keeper names no agent CLI to start.".to_owned())
    );
    let other = workspace_id('2');
    click_in(&format!("sidebar-workspace-{other}"), &mut cx);
    wait_drawn(&mut cx, "the other workspace on screen", |cx| {
        panes
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&other))
    });
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (_, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(
            &request,
            Request::OpenTerminal {
                workspace,
                program: Program::Shell,
                ..
            } if *workspace == other
        ),
        "{request:?}"
    );
}

#[gpui_kit::test]
fn the_shell_launcher_opens_a_shell_as_new_terminal_does(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    open_launcher_menu(&mut cx);
    wait_drawn(&mut cx, "the launchers", |cx| {
        drawn("terminal-launcher-shell", cx).is_some()
    });
    click_in("terminal-launcher-shell", &mut cx);
    let (workspace, _) = panes.answer_open(&terminal('a'), Program::Shell, &mut cx);
    assert_eq!(workspace, workspace_id('1'));
    wait_drawn(&mut cx, "the focus in the new terminal", |cx| {
        panes.focused_terminal(cx) == Some(terminal('a'))
    });
}

#[gpui_kit::test]
fn a_launcher_the_keeper_refuses_adds_no_terminal_and_the_reason_is_the_notice(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    open_launcher_menu(&mut cx);
    wait_drawn(&mut cx, "the launchers", |cx| {
        drawn("terminal-launcher-pi", cx).is_some()
    });
    click_in("terminal-launcher-pi", &mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(
        request,
        Request::OpenTerminal {
            program: Program::Agent { .. },
            ..
        }
    ));
    panes
        .keeper
        .reply(id, Err(refused("no pi on this machine")));
    wait_until(&mut cx, "the notice", |cx| {
        panes.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("no pi on this machine".to_owned())
    );
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, _| pane.terminals().is_empty())
    );
}

#[gpui_kit::test]
fn next_agent_cli_goes_through_every_cli_the_keeper_names_in_order_and_round(
    cx: &mut TestAppContext,
) {
    let third = AgentCli {
        agent: "third".to_owned(),
        title: "Third".to_owned(),
    };
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi(), third.clone()]);
    wait_for_launchers(&panes, 3, &mut cx);
    let mut seen = vec![chosen(&panes, &mut cx).expect("a chosen CLI")];
    for _ in 0..3 {
        panes.press_in_the_app::<NextAgentCli>(&mut cx);
        seen.push(chosen(&panes, &mut cx).expect("a chosen CLI"));
    }
    assert_eq!(seen, [claude(), pi(), third, claude()]);
    wait_for_chosen_mark(&["claude", "pi", "third"], "claude", &mut cx);
}

fn fill_with_lines(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> usize {
    let text: String = (0..200).map(|n| format!("line-{n}\r\n")).collect();
    panes.keeper.output(id, text.as_bytes());
    wait_until(cx, "the lines", |cx| {
        lines(&panes.pane, id, cx)
            .iter()
            .any(|line| line == "line-199")
    });
    top_line(panes, id, cx)
}

fn top_line(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> usize {
    let top = lines(&panes.pane, id, cx)[0].clone();
    top.strip_prefix("line-")
        .unwrap_or_else(|| panic!("the first line is {top:?}"))
        .parse()
        .expect("a number")
}

fn wheel(at: gpui_kit::Point<Pixels>, steps: f32, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_event(ScrollEvent {
        position: at,
        delta: ScrollDelta::Lines(point(0., steps)),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_wheel_over_the_launchers_scrolls_no_terminal_and_sends_its_program_nothing(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    let one = terminal('a');
    panes.open_terminal(&one, &mut cx);
    wait_drawn(&mut cx, "the launchers", |cx| {
        drawn("terminal-launchers", cx).is_some()
    });
    let newest = fill_with_lines(&panes, &one, &mut cx);
    let over_the_row = bounds_of("terminal-launchers", &mut cx).center();
    let over_the_terminal = panes.cell_centre(&one, 0, 1, 1, &mut cx);

    wheel(over_the_row, 1., &mut cx);
    assert_eq!(top_line(&panes, &one, &mut cx), newest);
    wheel(over_the_terminal, 1., &mut cx);
    assert_eq!(top_line(&panes, &one, &mut cx), newest - 3);
    wheel(over_the_terminal, -1., &mut cx);
    assert_eq!(top_line(&panes, &one, &mut cx), newest);

    panes
        .keeper
        .output(&one, b"\x1b[?1049h\x1b[H\x1b[2Kfull-screen");
    wait_until(&mut cx, "the alternate screen", |cx| {
        lines(&panes.pane, &one, cx)[0] == "full-screen"
    });
    wheel(over_the_row, 1., &mut cx);
    wheel(over_the_terminal, 1., &mut cx);
    assert_eq!(
        panes.keeper.input(&mut cx),
        (one.clone(), b"\x1b[A".to_vec())
    );
    cx.simulate_input("x");
    assert_eq!(panes.keeper.input(&mut cx), (one, b"x".to_vec()));
}

fn session(name: char, agent: &str, workspace: &WorkspaceId, resumable: bool) -> Session {
    Session {
        id: SessionId::new(name.to_string().repeat(32)),
        agent: agent.to_owned(),
        title: format!("{agent} {name}"),
        workspace: workspace.clone(),
        terminal: None,
        state: SessionState::NotRunning,
        turn: None,
        review: None,
        attached: None,
        resumable,
        turn_from_user: false,
    }
}

fn tell_agents(panes: &Panes, workspace: &WorkspaceId, sessions: Vec<Session>) {
    let shown = sessions.iter().map(|session| session.state).min();
    panes.keeper.tell(Fact::AgentsChanged(WorkspaceAgents {
        workspace: workspace.clone(),
        shown,
        sessions,
    }));
}

fn resume(session: &Session) -> Program {
    Program::Resume {
        session: session.id.clone(),
    }
}

fn offered(panes: &Panes, cx: &mut VisualTestContext) -> Vec<SessionId> {
    panes.pane.read_with(cx, |pane, cx| pane.resumable(cx))
}

#[gpui_kit::test]
fn a_session_listed_resumable_gets_a_resume_control_that_opens_its_resume_and_goes_when_listed_no_more(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let first_terminal = terminal('a');
    panes.open_terminal(&first_terminal, &mut cx);
    wait_until(&mut cx, "the first fit", |_| {
        panes.keeper.resizes(&first_terminal).len() == 1
    });
    let whole = panes.grid(&first_terminal);
    assert!(drawn("terminal-resume", &mut cx).is_none());
    assert!(offered(&panes, &mut cx).is_empty());

    let mut a = session('a', "claude", &one, true);
    a.title = "a".to_owned();
    let mut b = session('b', "pi", &one, true);
    b.title = "the long title of the second session".to_owned();
    let ended = session('c', "claude", &one, false);
    let elsewhere = session('d', "pi", &two, true);
    tell_agents(&panes, &one, vec![a.clone(), b.clone(), ended]);
    tell_agents(&panes, &two, vec![elsewhere.clone()]);
    wait_drawn(&mut cx, "the resume trigger", |cx| {
        drawn("terminal-resume", cx).is_some()
    });
    assert_eq!(offered(&panes, &mut cx), [a.id.clone(), b.id.clone()]);

    // The resume trigger sits in the single 40 px header bar and takes no height from the terminal.
    let (launchers, trigger) = (
        bounds_of("terminal-launchers", &mut cx),
        bounds_of("terminal-resume", &mut cx),
    );
    assert_eq!(launchers.size.height, px(40.));
    assert!(within(trigger, launchers));
    assert_eq!(panes.grid(&first_terminal), whole);
    assert!(launchers.bottom() <= bounds_of("terminal-slot-0", &mut cx).top());

    open_resume_menu(&mut cx);
    assert!(drawn("terminal-resume-2", &mut cx).is_none());
    let menu = bounds_of("terminal-resume-popover", &mut cx);
    let (first, second) = (
        bounds_of("terminal-resume-0", &mut cx),
        bounds_of("terminal-resume-1", &mut cx),
    );
    assert!(first.bottom() <= second.top());
    assert!(within(first, menu) && within(second, menu));
    let first_title = bounds_of("terminal-resume-title-0", &mut cx);
    let second_title = bounds_of("terminal-resume-title-1", &mut cx);
    assert!(
        second_title.size.width > first_title.size.width,
        "{second_title:?} is not wider than {first_title:?}: the control does not carry the title"
    );

    click_in("terminal-resume-1", &mut cx);
    let (workspace, asked) = panes.answer_open(&terminal('b'), resume(&b), &mut cx);
    assert_eq!((workspace, asked), (one.clone(), whole));
    wait_drawn(&mut cx, "the focus in the resume", |cx| {
        panes.focused_terminal(cx) == Some(terminal('b'))
    });

    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No session of this workspace is left to resume.".to_owned())
    );
    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    wait_drawn(&mut cx, "the other workspace on screen", |cx| {
        panes
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&two))
    });
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (_, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(
            &request,
            Request::OpenTerminal {
                workspace,
                program: Program::Shell,
                ..
            } if *workspace == two
        ),
        "{request:?}"
    );
    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    wait_drawn(&mut cx, "the first workspace on screen", |cx| {
        panes
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&one))
    });

    let mut running = a.clone();
    (running.resumable, running.terminal, running.state) =
        (false, Some(terminal('c')), SessionState::Works);
    tell_agents(&panes, &one, vec![running, b.clone()]);
    open_resume_menu(&mut cx);
    wait_drawn(&mut cx, "the first control gone", |cx| {
        drawn("terminal-resume-1", cx).is_none()
    });
    assert_eq!(offered(&panes, &mut cx), std::slice::from_ref(&b.id));
    assert!(drawn("terminal-resume-0", &mut cx).is_some());

    let other = format!("sidebar-workspace-{two}");
    click_in(&other, &mut cx);
    wait_drawn(&mut cx, "the other workspace", |cx| {
        offered(&panes, cx) == [elsewhere.id.clone()]
    });
    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    wait_drawn(&mut cx, "the first workspace again", |cx| {
        offered(&panes, cx) == [b.id.clone()]
    });

    tell_agents(&panes, &one, Vec::new());
    wait_drawn(&mut cx, "the trigger gone", |cx| {
        drawn("terminal-resume", cx).is_none()
    });
    assert!(offered(&panes, &mut cx).is_empty());

    tell_agents(&panes, &one, vec![b.clone()]);
    wait_drawn(&mut cx, "the offer again", |cx| {
        offered(&panes, cx) == [b.id.clone()]
    });
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('d'), resume(&b), &mut cx);

    if cfg!(target_os = "macos") {
        let next = session('e', "pi", &one, true);
        tell_agents(&panes, &one, vec![b.clone(), next.clone()]);
        wait_drawn(&mut cx, "the second session offered", |cx| {
            offered(&panes, cx) == [b.id.clone(), next.id.clone()]
        });
        panes.click_slot(0, &mut cx);
        assert_eq!(panes.focused_terminal(&mut cx), Some(terminal('d')));
        cx.simulate_keystrokes(common::key_of::<ResumeAgent>());
        panes.answer_open(&terminal('e'), resume(&next), &mut cx);
    }
}

#[gpui_kit::test]
fn a_resume_the_keeper_refuses_is_the_notice_and_the_session_can_be_resumed_again(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let one = workspace_id('1');
    let a = session('a', "claude", &one, true);
    tell_agents(&panes, &one, vec![a.clone()]);
    open_resume_menu(&mut cx);
    wait_drawn(&mut cx, "the resume item", |cx| {
        drawn("terminal-resume-0", cx).is_some()
    });

    click_in("terminal-resume-0", &mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(
        request,
        Request::OpenTerminal {
            program: Program::Resume { .. },
            ..
        }
    ));
    panes.keeper.reply(id, Err(refused("the session runs")));
    wait_until(&mut cx, "the notice", |cx| {
        panes.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, _| pane.terminals().is_empty())
    );

    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('b'), resume(&a), &mut cx);
}

#[gpui_kit::test]
fn a_resume_asked_for_once_is_not_asked_for_again_while_the_session_is_listed_resumable(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let a = session('a', "claude", &one, true);
    tell_agents(&panes, &one, vec![a.clone()]);

    // A second click before the answer asks nothing: the Keeper refuses a session only once it
    // runs, which is too late. Nor do a click or Resume Agent while the session is listed
    // resumable; the shell asked for in the other workspace is the barrier.
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('b'), resume(&a), &mut cx);
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);

    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    wait_drawn(&mut cx, "the other workspace on screen", |cx| {
        panes
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&two))
    });
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (_, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(
            &request,
            Request::OpenTerminal {
                workspace,
                program: Program::Shell,
                ..
            } if *workspace == two
        ),
        "{request:?}"
    );
}

fn wait_for_ending(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) {
    let view = panes
        .pane
        .read_with(cx, |pane, _| pane.terminal(id))
        .expect("the terminal");
    wait_until(cx, "the ending", |cx| {
        view.read_with(cx, |view, _| view.ending()).is_some()
    });
}

#[gpui_kit::test]
fn a_resume_whose_terminal_ends_before_the_session_reports_can_be_asked_for_again(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let one = workspace_id('1');
    let a = session('a', "claude", &one, true);
    tell_agents(&panes, &one, vec![a.clone()]);

    // The resumed program exits before it reports, as a CLI does that cannot find its session.
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('b'), resume(&a), &mut cx);
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: terminal('b'),
        ending: Ending::Exited(1),
    });
    wait_for_ending(&panes, &terminal('b'), &mut cx);
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: terminal('c'),
        ending: Ending::Signalled(9),
    });
    wait_for_ending(&panes, &terminal('c'), &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('d'), resume(&a), &mut cx);
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.notice()), None);
}

#[gpui_kit::test]
fn a_resume_whose_terminal_ended_before_the_answer_was_taken_can_be_asked_for_again(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let one = workspace_id('1');
    let a = session('a', "claude", &one, true);
    tell_agents(&panes, &one, vec![a.clone()]);

    // A program that fails at once can end before the pane takes the answer that named its
    // terminal: the end is sent first, then a change of the agents, then the answer.
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(
            &request,
            Request::OpenTerminal {
                program: Program::Resume { .. },
                ..
            }
        ),
        "{request:?}"
    );
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: terminal('b'),
        ending: Ending::Exited(1),
    });
    let marker = session('f', "pi", &one, true);
    tell_agents(&panes, &one, vec![a.clone(), marker.clone()]);
    wait_drawn(&mut cx, "the change heard", |cx| {
        offered(&panes, cx) == [a.id.clone(), marker.id.clone()]
    });
    panes.keeper.reply(
        id,
        Ok(Reply::TerminalOpened {
            terminal: terminal('b'),
        }),
    );
    wait_until(&mut cx, "the terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&terminal('b')))
    });
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
}

#[gpui_kit::test]
fn a_resume_is_held_until_its_own_terminal_ends_and_the_end_of_another_terminal_lets_none_go(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    panes.answer_open(&terminal('s'), Program::Shell, &mut cx);
    let a = session('a', "claude", &one, true);
    tell_agents(&panes, &one, vec![a.clone()]);
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('b'), resume(&a), &mut cx);

    // The shell ends before the resumed CLI reports: the resume is still held, so the click and
    // the key ask for nothing, as the shell then asked for in the other workspace shows.
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: terminal('s'),
        ending: Ending::Exited(0),
    });
    wait_for_ending(&panes, &terminal('s'), &mut cx);
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    wait_drawn(&mut cx, "the other workspace on screen", |cx| {
        panes
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&two))
    });
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (_, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(
            &request,
            Request::OpenTerminal {
                workspace,
                program: Program::Shell,
                ..
            } if *workspace == two
        ),
        "{request:?}"
    );

    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    wait_drawn(&mut cx, "the first workspace on screen", |cx| {
        panes
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&one))
    });
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: terminal('b'),
        ending: Ending::Exited(1),
    });
    wait_for_ending(&panes, &terminal('b'), &mut cx);
    open_resume_menu(&mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
}

#[gpui_kit::test]
fn resume_agent_takes_the_first_session_offered_that_no_resume_was_opened_for(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let one = workspace_id('1');
    let a = session('a', "claude", &one, true);
    let b = session('b', "pi", &one, true);
    tell_agents(&panes, &one, vec![a.clone(), b.clone()]);
    wait_drawn(&mut cx, "the two sessions offered", |cx| {
        offered(&panes, cx) == [a.id.clone(), b.id.clone()]
    });

    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('d'), resume(&b), &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No session of this workspace is left to resume.".to_owned())
    );
}

#[gpui_kit::test]
fn tab_close_button_closes_running_terminal_and_dismisses_ended_terminal_locally(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    let (t1, t2) = (terminal('1'), terminal('2'));

    panes.open_terminal(&t1, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    let close_t1 = format!("terminal-tab-{t1}-close");
    let close_t2 = format!("terminal-tab-{t2}-close");
    wait_drawn(&mut cx, "the terminal tabs inside the header bar", |cx| {
        drawn("terminal-launchers", cx).is_some()
            && drawn(&close_t1, cx).is_some()
            && drawn(&close_t2, cx).is_some()
    });

    // Open terminal tabs sit inside the single 40 px header bar.
    let launchers_row = bounds_of("terminal-launchers", &mut cx);
    assert_eq!(launchers_row.size.height, px(40.));
    assert!(within(bounds_of(&close_t1, &mut cx), launchers_row));

    // Clicking the close button on a running terminal sends CloseTerminal and removes it.
    click_in(&close_t1, &mut cx);
    let (req_id, request) = panes.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::CloseTerminal {
            terminal: t1.clone(),
        }
    );
    panes.keeper.reply(req_id, Ok(Reply::Done));
    wait_until(&mut cx, "the closed terminal removed", |cx| {
        panes.pane.read_with(cx, |pane, _| {
            pane.terminals() == std::slice::from_ref(&t2)
                && pane.shown() == std::slice::from_ref(&t2)
        })
    });

    // An ended terminal stays until closed, and closing it dismisses it locally with no request.
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: t2.clone(),
        ending: Ending::Exited(0),
    });
    wait_for_ending(&panes, &t2, &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        std::slice::from_ref(&t2)
    );
    click_in(&close_t2, &mut cx);
    wait_drawn(
        &mut cx,
        "the empty state after dismissing the last tab",
        |cx| {
            panes.pane.read_with(cx, |pane, _| {
                pane.terminals().is_empty() && pane.shown().is_empty()
            }) && drawn(&close_t2, cx).is_none()
                && drawn("terminal-how-to-open", cx).is_some()
        },
    );

    // Barrier: opening a new shell is the very next request the Keeper sees (no CloseTerminal was
    // sent for the already-ended terminal).
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (workspace, _) = panes.answer_open(&terminal('3'), Program::Shell, &mut cx);
    assert_eq!(workspace, workspace_id('1'));

    // CloseTerminal action closes the currently focused running terminal.
    panes.press_in_the_app::<CloseTerminal>(&mut cx);
    let (req_id, request) = panes.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::CloseTerminal {
            terminal: terminal('3'),
        }
    );
    panes.keeper.reply(req_id, Ok(Reply::Done));
    wait_until(&mut cx, "the terminal closed by action", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().is_empty())
    });
}

#[gpui_kit::test]
fn dismissing_resumable_sessions_from_the_popover_sends_dismiss_requests_for_one_or_all(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx) = open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let one = workspace_id('1');
    let a = session('a', "claude", &one, true);
    let b = session('b', "pi", &one, true);
    tell_agents(&panes, &one, vec![a.clone(), b.clone()]);
    open_resume_menu(&mut cx);

    click_in("terminal-resume-dismiss-0", &mut cx);
    let (req_id, request) = panes.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::DismissResumable {
            workspace: one.clone(),
            session: Some(a.id.clone()),
        }
    );
    panes.keeper.reply(req_id, Ok(Reply::Done));
    tell_agents(&panes, &one, vec![b.clone()]);
    wait_drawn(&mut cx, "one offer left", |cx| {
        offered(&panes, cx) == [b.id.clone()]
    });

    click_in("terminal-resume-dismiss-all", &mut cx);
    let (req_id, request) = panes.keeper.request(&mut cx);
    assert_eq!(
        request,
        Request::DismissResumable {
            workspace: one.clone(),
            session: None,
        }
    );
    panes.keeper.reply(req_id, Ok(Reply::Done));
    tell_agents(&panes, &one, Vec::new());
    wait_drawn(&mut cx, "resume trigger gone", |cx| {
        drawn("terminal-resume", cx).is_none() && drawn("terminal-resume-popover", cx).is_none()
    });
}

#[gpui_kit::test]
fn dock_popover_changes_terminal_placement_and_escape_closes_popovers(cx: &mut TestAppContext) {
    let (panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    open_dock_menu(&mut cx);
    click_in("terminal-place-bottom", &mut cx);
    wait_drawn(&mut cx, "dock popover closed", |cx| {
        drawn("terminal-place-popover", cx).is_none()
    });
    assert_eq!(
        panes
            .models
            .settings
            .read_with(&cx, |settings, _| settings.terminal_place()),
        TerminalPlace::Below
    );

    open_dock_menu(&mut cx);
    click_in("terminal-place-left", &mut cx);
    wait_drawn(&mut cx, "dock popover closed after left", |cx| {
        drawn("terminal-place-popover", cx).is_none()
    });
    assert_eq!(
        panes
            .models
            .settings
            .read_with(&cx, |settings, _| settings.terminal_place()),
        TerminalPlace::Left
    );

    open_launcher_menu(&mut cx);
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    wait_drawn(&mut cx, "launcher popover dismissed by escape", |cx| {
        drawn("terminal-launcher-popover", cx).is_none()
    });
}

#[gpui_kit::test]
fn many_open_tabs_stay_on_one_40px_row_without_clipping_right_controls(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    for ch in ['1', '2', '3', '4', '5', '6', '7', '8'] {
        panes.open_terminal(&terminal(ch), &mut cx);
    }
    wait_drawn(&mut cx, "all right controls remain visible", |cx| {
        drawn("terminal-launchers", cx).is_some()
            && drawn("terminal-launcher-split", cx).is_some()
            && drawn("terminal-split-toggle", cx).is_some()
            && drawn("terminal-place", cx).is_some()
    });
    let bar = bounds_of("terminal-launchers", &mut cx);
    assert_eq!(bar.size.height, px(40.));
    for selector in [
        "terminal-launcher-split",
        "terminal-split-toggle",
        "terminal-place",
    ] {
        let bounds = bounds_of(selector, &mut cx);
        assert!(
            within(bounds, bar),
            "{selector} ({bounds:?}) fell outside {bar:?}"
        );
    }
}

#[gpui_kit::test]
fn empty_state_launcher_button_opens_the_chosen_agent_cli(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    wait_for_launchers(&panes, 2, &mut cx);
    wait_drawn(&mut cx, "the empty-state agent launcher", |cx| {
        drawn("terminal-empty-open-claude", cx).is_some()
            && drawn("terminal-empty-open-pi", cx).is_some()
            && drawn("terminal-empty-open-shell", cx).is_some()
    });

    click_in("terminal-empty-open-claude", &mut cx);
    let (workspace, _) = panes.answer_open(&terminal('a'), agent("claude"), &mut cx);
    assert_eq!(workspace, workspace_id('1'));
    wait_drawn(&mut cx, "the agent terminal shown", |cx| {
        panes.pane.read_with(cx, |pane, _| {
            pane.shown() == std::slice::from_ref(&terminal('a'))
        }) && drawn("terminal-how-to-open", cx).is_none()
    });
}

#[gpui_kit::test]
fn pressing_enter_in_empty_focused_terminal_pane_launches_chosen_agent_cli(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_naming(cx, vec![claude(), pi()]);
    wait_for_launchers(&panes, 2, &mut cx);
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    cx.run_until_parked();
    assert!(cx.update(|window, _| panes.pane_focus.is_focused(window)));

    cx.simulate_keystrokes("enter");
    let (workspace, _) = panes.answer_open(&terminal('a'), agent("claude"), &mut cx);
    assert_eq!(workspace, workspace_id('1'));
    wait_drawn(&mut cx, "the launched agent terminal", |cx| {
        panes.pane.read_with(cx, |pane, _| {
            pane.shown() == std::slice::from_ref(&terminal('a'))
        })
    });
}

#[gpui_kit::test]
fn pressing_enter_in_empty_focused_terminal_pane_falls_back_to_shell_without_clis(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_naming(cx, Vec::new());
    wait_drawn(&mut cx, "the empty state", |cx| {
        drawn("terminal-how-to-open", cx).is_some()
    });
    cx.update(|window, cx| panes.pane_focus.focus(window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("enter");
    let (workspace, _) = panes.answer_open(&terminal('s'), Program::Shell, &mut cx);
    assert_eq!(workspace, workspace_id('1'));
    wait_drawn(&mut cx, "the launched shell terminal", |cx| {
        panes.pane.read_with(cx, |pane, _| {
            pane.shown() == std::slice::from_ref(&terminal('s'))
        })
    });
}
