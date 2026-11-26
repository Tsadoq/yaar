//! Component test: the control that resumes a session a restart cut off, with a stand-in agent.

mod common;

use gpui_kit::{Modifiers, TestAppContext};
use yaar_link::{Program, Request, SessionState};
use yaar_system_tests::standin::{Did, Script, Step};
use yaar_system_tests::{DEADLINE, World, wait_until as wait_for};

use common::{a_session_cut_off, drawn, wait_until};

#[gpui_kit::test]
fn the_resume_control_of_a_session_a_restart_cut_off_runs_its_cli_s_resume_and_goes_when_it_reports(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    let cut_off = a_session_cut_off(&mut world);
    let (mut window, mut cx) = common::open_composed_in(world, cx, Vec::new());
    window.open_this_folder(cut_off.repo.path(), &mut cx);

    wait_until(&mut cx, "the control to resume the session", |cx| {
        drawn("terminal-resume", cx).is_some()
    });
    assert_eq!(
        window
            .terminals
            .read_with(&cx, |pane, cx| pane.resumable(cx)),
        std::slice::from_ref(&cut_off.session)
    );
    let trigger = window.point_in("terminal-resume", &mut cx);
    cx.simulate_click(trigger, Modifiers::none());
    wait_until(&mut cx, "the resume menu", |cx| {
        drawn("terminal-resume-0", cx).is_some()
    });
    assert!(drawn("terminal-resume-1", &mut cx).is_none(), "one control");
    assert_eq!(
        window.terminals.read_with(&cx, |pane, _| pane.terminals()),
        [],
        "nothing runs before the click"
    );

    // The resumed stand-in is held before it reports its start, so the order is the test's own.
    let start_gate = window.world.home().with_file_name("resume-start-gate");
    window.world.agent_script(
        "default",
        &Script(vec![
            Step::HoldAtStart(start_gate.clone()),
            Step::Hold(cut_off.gate.clone()),
            Step::Exit,
        ]),
    );

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

    // The stand-in is held, so this look finds the terminal open and the session not yet reported.
    let listed = window.models.agents.read_with(&cx, |agents, _| {
        agents
            .session(&cut_off.session)
            .is_some_and(|session| session.resumable)
    });
    let trigger_shown = drawn("terminal-resume", &mut cx).is_some();
    assert!(
        listed && trigger_shown,
        "no look after the click found the session listed with its trigger: listed {listed}, trigger {trigger_shown}"
    );

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
            let trigger_shown = drawn("terminal-resume", cx).is_some();
            assert!(
                !listed || trigger_shown,
                "the control went before the session was listed resumable no more"
            );
            !listed
        },
    );

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

    wait_until(&mut cx, "the resumed session to run", |cx| {
        window.models.agents.read_with(cx, |agents, _| {
            agents.session(&cut_off.session).is_some_and(|session| {
                !session.resumable && session.state != SessionState::NotRunning
            })
        })
    });
    wait_until(&mut cx, "the control gone", |cx| {
        drawn("terminal-resume", cx).is_none()
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
