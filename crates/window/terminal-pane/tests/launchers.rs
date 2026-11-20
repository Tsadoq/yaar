//! Component test: the launchers next to the shell and the controls that resume the
//! sessions a restart cut off, on GPUI's test platform with no window on any screen. The
//! test plays the Keeper on the other end of a socket pair: it names the agent CLIs, tells
//! which sessions are resumable and answers what the pane asks. Keys, clicks and the
//! wheel go in; the pane is read back through the model it shows and the elements it
//! draws.
//!
//! A negative clause is read after a barrier: the next request the Keeper hears, caused
//! through the same path after the key under test, shows that the key sent nothing.

mod common;

use gpui_kit::{
    Bounds, Modifiers, Pixels, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point,
};
use yaar_link::{
    AgentCli, Ending, Fact, Program, Reply, Repository, Request, Session, SessionId, SessionState,
    TerminalId, WorkspaceAgents, WorkspaceId,
};
use yaar_terminal_pane::{NewAgentTerminal, NewTerminal, NextAgentCli, ResumeAgent};

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

/// Titles that differ from the names of the agents, and in length the other way round
/// for the two CLIs: the launcher asks the Keeper for the agent's name, and the words on
/// it are the title, which the widths of the two launchers tell apart.
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

/// A window whose Keeper names `clis` and lists one workspace, on screen. The Window asks
/// for the CLIs when it connects, so they are set before.
fn open_naming(cx: &mut TestAppContext, clis: Vec<AgentCli>) -> (Panes, VisualTestContext) {
    open_naming_in(cx, common::one_repository(), clis)
}

/// [`open_naming`] with the Keeper listing `repository`.
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

/// Waits until the pane offers `count` launchers, which the Keeper's answer to the
/// question for its CLIs brings.
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

/// Waits until the mark of the chosen launcher is drawn over `agent` and over no other
/// of `all`, and gives the bounds of the mark.
fn wait_for_chosen_mark(all: &[&str], agent: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
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

/// Whether `inner` lies inside `outer`.
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

    // The shell's control and one for each CLI, left to right in the Keeper's order, in
    // one row at the top of the pane that lies above the line that says how to open a
    // terminal.
    wait_drawn(&mut cx, "the launchers", |cx| {
        drawn("terminal-launcher-pi", cx).is_some()
    });
    let row = bounds_of("terminal-launchers", &mut cx);
    let (shell, first, second) = (
        bounds_of("terminal-launcher-shell", &mut cx),
        bounds_of("terminal-launcher-claude", &mut cx),
        bounds_of("terminal-launcher-pi", &mut cx),
    );
    assert!(shell.right() <= first.left() && first.right() <= second.left());
    // Each launcher carries its CLI's title: pi's is longer than Claude Code's and its
    // name is shorter.
    assert!(
        second.size.width > first.size.width,
        "{second:?} is not wider than {first:?}: the launcher does not carry the CLI's title"
    );
    for launcher in [shell, first, second] {
        assert!(within(launcher, row), "{launcher:?} lies outside {row:?}");
    }
    let how_to_open = bounds_of("terminal-how-to-open", &mut cx);
    assert!(row.bottom() <= how_to_open.top());
    // The row is the top of the pane: the pane beside it starts at the same height.
    assert_eq!(row.top(), bounds_of("elsewhere-pane", &mut cx).top());

    // The first CLI is the chosen one, and only it carries the mark, over its launcher.
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    let mark = wait_for_chosen_mark(&["claude", "pi"], "claude", &mut cx);
    assert!(within(mark, first));
    assert!(drawn("terminal-launcher-chosen-shell", &mut cx).is_none());

    // The row lies above the first terminal's slot, not over it.
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

    // A click on the launcher of Claude Code asks for a terminal that runs the agent by
    // its name, not its title.
    click_in("terminal-launcher-claude", &mut cx);
    panes.answer_open(&terminal('x'), agent("claude"), &mut cx);

    // A click on the launcher of pi asks for a terminal that runs pi, at the size the
    // pane lays terminals out at; the answer is shown and has the focus.
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
    // A click chooses nothing.
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));

    // New Agent Terminal opens the chosen one.
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('c'), agent("claude"), &mut cx);

    // Next Agent CLI makes pi the chosen one, as the mark shows, and sends nothing; New
    // Agent Terminal then opens pi.
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(pi()));
    let mark = wait_for_chosen_mark(&["claude", "pi"], "pi", &mut cx);
    assert!(within(mark, bounds_of("terminal-launcher-pi", &mut cx)));
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('d'), agent("pi"), &mut cx);

    // After the last the first is chosen again, and twice more brings it back to the same
    // place by way of pi.
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    wait_for_chosen_mark(&["claude", "pi"], "claude", &mut cx);
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(pi()));
    panes.press_in_the_app::<NextAgentCli>(&mut cx);
    assert_eq!(chosen(&panes, &mut cx), Some(claude()));
    panes.press_in_the_app::<NewAgentTerminal>(&mut cx);
    panes.answer_open(&terminal('e'), agent("claude"), &mut cx);

    // The keys are kept on macOS, where they work with the focus in a terminal too.
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
fn with_no_cli_named_the_pane_offers_only_the_shell_and_the_key_says_so(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_naming_in(cx, two_workspaces(), Vec::new());
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

    // The key opens nothing and says why, and Next Agent CLI has nothing to move to. A key
    // that fell back to a shell would ask for one in the workspace on screen, which is the
    // one the barrier cannot be: the barrier is a shell asked for by New Terminal in the
    // other workspace, so that the first request the Keeper hears is told from anything the
    // two keys sent.
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

/// Fills the screen of a terminal with numbered lines, so that it has a history, and says
/// the number on its first line.
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

    // A step up over the row moves nothing. A step up over the terminal after it, which
    // is the same path, moves the history by one step of three lines and no more: the
    // step over the row was not kept.
    wheel(over_the_row, 1., &mut cx);
    assert_eq!(top_line(&panes, &one, &mut cx), newest);
    wheel(over_the_terminal, 1., &mut cx);
    assert_eq!(top_line(&panes, &one, &mut cx), newest - 3);
    wheel(over_the_terminal, -1., &mut cx);
    assert_eq!(top_line(&panes, &one, &mut cx), newest);

    // On the alternate screen a step of the wheel is an arrow key for the program: none
    // goes for the one over the row, and the first the program gets is the one over the
    // terminal.
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

/// A session of `agent` in `workspace` that no agent runs, resumable or not.
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

/// Tells the Window the sessions of one workspace, as the Keeper does when they change.
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

    // Two sessions of the workspace on screen are resumable, one that ended is not, and
    // one of the other workspace is resumable there.
    // The titles differ in length the other way round from the agents' names, which the
    // widths of the controls below tell apart: the words are `Resume` and the title.
    let mut a = session('a', "claude", &one, true);
    a.title = "a".to_owned();
    let mut b = session('b', "pi", &one, true);
    b.title = "the long title of the second session".to_owned();
    let ended = session('c', "claude", &one, false);
    let elsewhere = session('d', "pi", &two, true);
    tell_agents(&panes, &one, vec![a.clone(), b.clone(), ended]);
    tell_agents(&panes, &two, vec![elsewhere.clone()]);
    wait_drawn(&mut cx, "the resume line", |cx| {
        drawn("terminal-resume-1", cx).is_some()
    });
    assert_eq!(offered(&panes, &mut cx), [a.id.clone(), b.id.clone()]);
    assert!(drawn("terminal-resume-2", &mut cx).is_none());

    // The line lies under the launchers and above the terminal, and takes its room: the
    // terminal was resized to fewer rows, as when the window is made smaller.
    let (launchers, line) = (
        bounds_of("terminal-launchers", &mut cx),
        bounds_of("terminal-resume", &mut cx),
    );
    assert!(launchers.bottom() <= line.top());
    let (first, second) = (
        bounds_of("terminal-resume-0", &mut cx),
        bounds_of("terminal-resume-1", &mut cx),
    );
    assert!(first.right() <= second.left());
    assert!(within(first, line) && within(second, line));
    assert!(
        second.size.width > first.size.width,
        "{second:?} is not wider than {first:?}: the control does not carry the title"
    );
    wait_until(&mut cx, "the terminal made shorter", |_| {
        panes
            .keeper
            .resizes(&first_terminal)
            .last()
            .is_some_and(|last| last.rows < whole.rows)
    });
    let shorter = panes.grid(&first_terminal);
    assert_eq!(shorter.columns, whole.columns);
    assert!(shorter.rows < whole.rows, "{shorter:?} and {whole:?}");
    assert!(line.bottom() <= bounds_of("terminal-slot-0", &mut cx).top());

    // A click on the second control asks for a terminal that runs that session's resume,
    // and the answer is shown and has the focus.
    click_in("terminal-resume-1", &mut cx);
    let (workspace, asked) = panes.answer_open(&terminal('b'), resume(&b), &mut cx);
    assert_eq!((workspace, asked), (one.clone(), shorter));
    wait_drawn(&mut cx, "the focus in the resume", |cx| {
        panes.focused_terminal(cx) == Some(terminal('b'))
    });

    // Resume Agent takes the first session it has opened no resume for, and so the
    // second press reaches no session twice; with none left it sends nothing and says so.
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No session of this workspace is left to resume.".to_owned())
    );
    // The barrier is a shell asked for in the other workspace, which is the next request
    // the Keeper hears when the key sent nothing: a key that fell back to a shell would
    // ask for one in the workspace on screen.
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

    // The resumed session reports: the Keeper tells it is no longer resumable, and its
    // control goes while the other stays.
    let mut running = a.clone();
    (running.resumable, running.terminal, running.state) =
        (false, Some(terminal('c')), SessionState::Works);
    tell_agents(&panes, &one, vec![running, b.clone()]);
    wait_drawn(&mut cx, "the first control gone", |cx| {
        drawn("terminal-resume-1", cx).is_none()
    });
    assert_eq!(offered(&panes, &mut cx), std::slice::from_ref(&b.id));
    assert!(drawn("terminal-resume-0", &mut cx).is_some());

    // The other workspace on screen has its own line.
    let other = format!("sidebar-workspace-{two}");
    click_in(&other, &mut cx);
    wait_drawn(&mut cx, "the other workspace", |cx| {
        offered(&panes, cx) == [elsewhere.id.clone()]
    });
    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    wait_drawn(&mut cx, "the first workspace again", |cx| {
        offered(&panes, cx) == [b.id.clone()]
    });

    // The last session goes: the line goes with it, and the terminals have their rows
    // back.
    tell_agents(&panes, &one, Vec::new());
    wait_drawn(&mut cx, "the line gone", |cx| {
        drawn("terminal-resume", cx).is_none()
    });
    assert!(offered(&panes, &mut cx).is_empty());
    // The terminal shown now was opened under the line and fits the whole room again
    // (the notice of the key that found nothing to resume took rows from it before).
    let shown = terminal('c');
    wait_until(&mut cx, "the terminal made taller", |_| {
        panes.keeper.resizes(&shown).last() == Some(&whole)
    });

    // A restart lists the session resumable again: what the pane remembered of its
    // resume is gone, and Resume Agent opens it once more.
    tell_agents(&panes, &one, vec![b.clone()]);
    wait_drawn(&mut cx, "the offer again", |cx| {
        offered(&panes, cx) == [b.id.clone()]
    });
    panes.press_in_the_app::<ResumeAgent>(&mut cx);
    panes.answer_open(&terminal('d'), resume(&b), &mut cx);

    // The key is kept on macOS, where it works with the focus in a terminal too.
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
    wait_drawn(&mut cx, "the resume line", |cx| {
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

    // The refusal did not use up the session: the key takes it again.
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
    wait_drawn(&mut cx, "the resume line", |cx| {
        drawn("terminal-resume-0", cx).is_some()
    });

    // A second click before the Keeper has answered the first asks nothing: the Keeper
    // refuses a session only once it runs, which is later than that.
    click_in("terminal-resume-0", &mut cx);
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('b'), resume(&a), &mut cx);
    // Nor does a click after the answer, while the Keeper still lists the session as
    // resumable, or Resume Agent.
    click_in("terminal-resume-0", &mut cx);
    panes.press_in_the_app::<ResumeAgent>(&mut cx);

    // The barrier is a shell asked for in the other workspace: it is the next request the
    // Keeper hears when none of the above asked for a second resume.
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

/// Waits until the view of `id` shows that its program ended.
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
    wait_drawn(&mut cx, "the resume line", |cx| {
        drawn("terminal-resume-0", cx).is_some()
    });

    // The resumed program exits before it reports, as a CLI does that cannot find its
    // session: the Keeper still lists the session resumable, and the pane, which holds
    // no terminal for it that runs, asks again for a click and for the key.
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('b'), resume(&a), &mut cx);
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: terminal('b'),
        ending: Ending::Exited(1),
    });
    wait_for_ending(&panes, &terminal('b'), &mut cx);
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
    wait_drawn(&mut cx, "the resume line", |cx| {
        drawn("terminal-resume-0", cx).is_some()
    });

    // A program that fails at once can end before the pane takes the answer that named
    // its terminal. The fact for the end is sent first, and a change of the agents
    // behind it, which the pane shows only after it has heard the end, stands for the
    // moment the pane took it; then the answer.
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
    click_in("terminal-resume-0", &mut cx);
    panes.answer_open(&terminal('c'), resume(&a), &mut cx);
}
