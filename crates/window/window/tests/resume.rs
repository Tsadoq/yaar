//! Component test: the control that resumes a session a restart cut off, in the composed
//! Window on GPUI's test platform, with no window on any screen.
//!
//! The Keeper is the built program, started again after everything was killed as a restart of
//! the machine would; the agent beneath it is the stand-in as Claude Code, started by the
//! control. What reached the stand-in is read from the file it writes down, and the control
//! from the terminal pane's model and from what the pane draws.

mod common;

use gpui_kit::{Modifiers, TestAppContext};
use yaar_link::{Program, Request, SessionState};
use yaar_system_tests::standin::{Did, Script, Step};
use yaar_system_tests::{DEADLINE, World, wait_until as wait_for};

use common::{a_session_cut_off, drawn, wait_until};

// AGENT-15 (the Window's control; FR-120).
#[gpui_kit::test]
fn the_resume_control_of_a_session_a_restart_cut_off_runs_its_cli_s_resume_and_goes_when_it_reports(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    let cut_off = a_session_cut_off(&mut world);
    let (mut window, mut cx) = common::open_composed_in(world, cx, Vec::new());
    window.open_this_folder(cut_off.repo.path(), &mut cx);

    // After the restart the pane offers the session, one control for it, with no terminal
    // open and no stand-in running.
    wait_until(&mut cx, "the control to resume the session", |cx| {
        drawn("terminal-resume-0", cx).is_some()
    });
    assert_eq!(
        window
            .terminals
            .read_with(&cx, |pane, cx| pane.resumable(cx)),
        std::slice::from_ref(&cut_off.session)
    );
    assert!(drawn("terminal-resume-1", &mut cx).is_none(), "one control");
    assert_eq!(
        window.terminals.read_with(&cx, |pane, _| pane.terminals()),
        [],
        "nothing runs before the click"
    );

    // The resumed stand-in is held before it reports its start, so that the order of what
    // follows is made by the test and not by how fast a login shell, the stand-in and its
    // hook program start: until the test lets it go, the session has not reported.
    let start_gate = window.world.home().with_file_name("resume-start-gate");
    window.world.agent_script(
        "default",
        &Script(vec![
            Step::HoldAtStart(start_gate.clone()),
            Step::Hold(cut_off.gate.clone()),
            Step::Exit,
        ]),
    );

    // A click runs the CLI's own resume command with the session's id: the stand-in is
    // started with exactly those arguments.
    let at = window.point_in("terminal-resume-0", &mut cx);
    cx.simulate_click(at, Modifiers::none());
    let wanted = ["--resume".to_owned(), cut_off.cli_session.clone()];
    assert!(
        wait_for(DEADLINE, || window.world.agent_log("default").args
            == wanted),
        "the stand-in was not started with {wanted:?}: {:?}",
        window.world.agent_log("default")
    );
    assert_eq!(
        window.world.agent_log("default").session,
        cut_off.cli_session
    );
    wait_until(&mut cx, "the terminal the click opened", |cx| {
        window
            .terminals
            .read_with(cx, |pane, _| pane.terminals().len() == 1)
    });
    assert!(
        window
            .world
            .agent_log("default")
            .did
            .iter()
            .all(|did| !matches!(did, Did::Hook { .. })),
        "the stand-in reported while it was held: {:?}",
        window.world.agent_log("default")
    );

    // The line stands for as long as the Keeper lists the session resumable, not only until
    // the click: with the terminal open and the stand-in running but not yet reported, the
    // Keeper lists the session resumable and the control is drawn. This look is taken
    // while the stand-in is held, so it is a look that finds both, whatever the speed of the
    // machine.
    let listed = window.models.agents.read_with(&cx, |agents, _| {
        agents
            .session(&cut_off.session)
            .is_some_and(|session| session.resumable)
    });
    let line = drawn("terminal-resume-0", &mut cx).is_some();
    assert!(
        listed && line,
        "no look after the click found the session listed with its line: listed {listed}, line {line}"
    );

    // The stand-in goes on and reports: from then on, whenever the model still lists the
    // session resumable, the control is drawn, and it goes when the session is listed so
    // no more.
    std::fs::write(&start_gate, "").expect("let the stand-in report");
    wait_until(
        &mut cx,
        "the session to be listed resumable no more",
        |cx| {
            let listed = window.models.agents.read_with(cx, |agents, _| {
                agents
                    .session(&cut_off.session)
                    .is_some_and(|session| session.resumable)
            });
            let line = drawn("terminal-resume-0", cx).is_some();
            assert!(
                !listed || line,
                "the control went before the session was listed resumable no more"
            );
            !listed
        },
    );

    // One click asked for one resume, and the Keeper refused nothing.
    let resumes = window
        .tap
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                Request::OpenTerminal {
                    program: Program::Resume { .. },
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert!(
        matches!(
            resumes.as_slice(),
            [Request::OpenTerminal { program: Program::Resume { session }, .. }]
                if *session == cut_off.session
        ),
        "one click sends one resume of the session: {resumes:?}"
    );

    // The session reports: the Keeper lists it running in the terminal the control opened, and
    // the line goes. No other session came.
    wait_until(&mut cx, "the resumed session to run", |cx| {
        window.models.agents.read_with(cx, |agents, _| {
            agents.session(&cut_off.session).is_some_and(|session| {
                !session.resumable && session.state != SessionState::NotRunning
            })
        })
    });
    wait_until(&mut cx, "the control gone", |cx| {
        drawn("terminal-resume-0", cx).is_none()
    });
    let opened = window.terminals.read_with(&cx, |pane, _| pane.terminals());
    assert_eq!(opened.len(), 1, "the click opened one terminal: {opened:?}");
    let session = window
        .models
        .agents
        .read_with(&cx, |agents, _| agents.session(&cut_off.session).cloned())
        .expect("the agents' list still holds the session");
    assert_eq!(session.terminal.as_ref(), Some(&opened[0]));
    assert_eq!(
        window.terminals.read_with(&cx, |pane, _| pane.notice()),
        None,
        "the Keeper refused nothing the click asked"
    );
    assert_eq!(
        window.models.agents.read_with(&cx, |agents, _| agents
            .of(&cut_off.workspace)
            .map(|agents| agents.sessions.len())),
        Some(1),
        "the same session runs again, and no other came"
    );
    assert_eq!(
        window
            .terminals
            .read_with(&cx, |pane, cx| pane.resumable(cx)),
        []
    );

    // The stand-in ends at the gate, so that nothing of the test is left running.
    std::fs::write(&cut_off.gate, "").expect("open the gate");
    assert!(
        wait_for(DEADLINE, || window
            .world
            .agent_log("default")
            .did
            .iter()
            .any(|did| matches!(did, Did::Exited { .. }))),
        "the resumed stand-in did not end at the gate"
    );
}
