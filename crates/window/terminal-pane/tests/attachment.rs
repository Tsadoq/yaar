//! A negative clause is read after a barrier: something the test causes afterwards through the
//! same path, whose arrival shows that everything before has arrived (a letter typed into the
//! terminal, or the next request).

mod common;

use gpui_kit::{
    Modifiers, MouseButton, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point,
};
use yaar_frame::Wanted;
use yaar_link::{
    Attachment, Fact, Program, Reply, Request, ReviewId, Session, SessionId, SessionState,
    TerminalId, TerminalInfo, ThreadId, WorkspaceAgents, WorkspaceId,
};
use yaar_terminal_pane::{
    ADDRESS_EACH_COMMENT, AddressEachComment, DetachAttachment, NewTerminal, ToggleSplit,
    ViewAttachment,
};

use common::{
    Panes, click_in, drawn, key_of, lines, open_panes, terminal, wait_drawn, wait_until,
    workspace_id,
};

fn review(name: char) -> ReviewId {
    ReviewId::new(name.to_string().repeat(32))
}

fn session(
    name: char,
    workspace: &WorkspaceId,
    terminal: &TerminalId,
    state: SessionState,
    attached: Option<Attachment>,
) -> Session {
    Session {
        id: SessionId::new(name.to_string().repeat(32)),
        agent: "claude".to_owned(),
        title: format!("Claude Code {name}"),
        workspace: workspace.clone(),
        terminal: Some(terminal.clone()),
        state,
        turn: None,
        review: Some(review(name)),
        attached,
        resumable: false,
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

fn wait_for(selector: &str, drawn_now: bool, cx: &mut VisualTestContext) {
    wait_drawn(cx, selector, |cx| {
        drawn(selector, cx).is_some() == drawn_now
    });
}

fn offers_address(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> bool {
    panes
        .pane
        .read_with(cx, |pane, cx| pane.offers_address(id, cx))
}

fn viewed(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> Option<String> {
    panes
        .pane
        .read_with(cx, |pane, _| pane.viewed_attachment(id))
}

fn attachment(panes: &Panes, id: &TerminalId, cx: &mut VisualTestContext) -> Option<Attachment> {
    panes.pane.read_with(cx, |pane, cx| pane.attachment(id, cx))
}

fn answer_text(panes: &Panes, review: &ReviewId, text: &str, cx: &mut VisualTestContext) {
    let (id, request) = panes.keeper.request(cx);
    assert_eq!(
        request,
        Request::GetAttachment {
            review: review.clone()
        }
    );
    panes.keeper.reply(
        id,
        Ok(Reply::Attachment {
            text: text.to_owned(),
        }),
    );
}

fn answer_detach(panes: &Panes, review: &ReviewId, cx: &mut VisualTestContext) {
    let (id, request) = panes.keeper.request(cx);
    assert_eq!(
        request,
        Request::DetachReview {
            review: review.clone()
        }
    );
    panes.keeper.reply(id, Ok(Reply::Done));
}

fn nothing_was_sent(panes: &mut Panes, id: &TerminalId, cx: &mut VisualTestContext) {
    panes.click_slot(0, cx);
    cx.simulate_input("z");
    assert_eq!(panes.keeper.input_until(id, b"z", cx), b"");
}

fn no_other_request_was_sent(panes: &mut Panes, cx: &mut VisualTestContext) {
    panes.press_in_the_app::<NewTerminal>(cx);
    let (id, request) = panes.keeper.request(cx);
    assert!(
        matches!(request, Request::OpenTerminal { .. }),
        "the Window asked {request:?}"
    );
    panes.keeper.reply(
        id,
        Err(common::refused("this test needs no second terminal")),
    );
}

fn one_agent(
    cx: &mut TestAppContext,
    state: SessionState,
    attached: Option<Attachment>,
) -> (Panes, VisualTestContext, TerminalId) {
    let (mut panes, mut cx) = open_panes(cx);
    let t = terminal('a');
    panes.open_terminal(&t, &mut cx);
    tell_agents(
        &panes,
        &workspace_id('1'),
        vec![session('x', &workspace_id('1'), &t, state, attached)],
    );
    (panes, cx, t)
}

fn typed_since(panes: &Panes, id: &TerminalId, before: usize) -> Vec<Vec<u8>> {
    panes.keeper.inputs(id)[before..].to_vec()
}

#[gpui_kit::test]
fn an_attached_review_shows_at_the_agents_terminal_with_a_control_to_view_what_will_be_sent_and_one_to_detach_it(
    cx: &mut TestAppContext,
) {
    let one = workspace_id('1');
    let (mut panes, mut cx) = open_panes(cx);
    let t = terminal('a');
    panes.open_terminal(&t, &mut cx);

    // Nothing is attached, so no line is drawn. The waiting mark, which arrives through the
    // same agents the line is read from, is the barrier.
    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::WaitsForPermission,
            None,
        )],
    );
    wait_for("terminal-waiting-0", true, &mut cx);
    assert_eq!(attachment(&panes, &t, &mut cx), None);
    assert!(drawn("terminal-attachment-0", &mut cx).is_none());

    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::Idle,
            Some(Attachment::Review),
        )],
    );
    wait_for("terminal-attachment-0", true, &mut cx);
    assert_eq!(attachment(&panes, &t, &mut cx), Some(Attachment::Review));
    assert!(drawn("terminal-attachment-view-0", &mut cx).is_some());
    assert!(drawn("terminal-attachment-detach-0", &mut cx).is_some());
    assert!(drawn("terminal-attachment-text-0", &mut cx).is_none());
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    click_in("terminal-attachment-view-0", &mut cx);
    answer_text(&panes, &review('x'), "T", &mut cx);
    wait_until(&mut cx, "the text of the attachment", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.viewed_attachment(&t).is_some())
    });
    assert_eq!(viewed(&panes, &t, &mut cx), Some("T".to_owned()));
    wait_for("terminal-attachment-text-0", true, &mut cx);

    click_in("terminal-attachment-view-0", &mut cx);
    wait_for("terminal-attachment-text-0", false, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    panes.press_in_the_app::<ViewAttachment>(&mut cx);
    answer_text(&panes, &review('x'), "T", &mut cx);
    wait_for("terminal-attachment-text-0", true, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), Some("T".to_owned()));
    panes.press_in_the_app::<ViewAttachment>(&mut cx);
    wait_for("terminal-attachment-text-0", false, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    // The key is kept on macOS, where it works with the focus in the terminal too.
    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        assert_eq!(panes.focused_terminal(&mut cx), Some(t.clone()));
        cx.simulate_keystrokes(key_of::<ViewAttachment>());
        answer_text(&panes, &review('x'), "T", &mut cx);
        wait_for("terminal-attachment-text-0", true, &mut cx);
        cx.simulate_keystrokes(key_of::<ViewAttachment>());
        wait_for("terminal-attachment-text-0", false, &mut cx);
    }

    click_in("terminal-attachment-detach-0", &mut cx);
    answer_detach(&panes, &review('x'), &mut cx);
    no_other_request_was_sent(&mut panes, &mut cx);
    panes.press_in_the_app::<DetachAttachment>(&mut cx);
    answer_detach(&panes, &review('x'), &mut cx);
    no_other_request_was_sent(&mut panes, &mut cx);
    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        cx.simulate_keystrokes(key_of::<DetachAttachment>());
        answer_detach(&panes, &review('x'), &mut cx);
        no_other_request_was_sent(&mut panes, &mut cx);
    }

    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, &t, SessionState::Idle, None)],
    );
    wait_for("terminal-attachment-0", false, &mut cx);
    assert_eq!(attachment(&panes, &t, &mut cx), None);
    assert!(drawn("terminal-attachment-view-0", &mut cx).is_none());
    assert!(drawn("terminal-attachment-detach-0", &mut cx).is_none());

    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::Idle,
            Some(Attachment::Thread(ThreadId::new("b".repeat(32)))),
        )],
    );
    wait_for("terminal-attachment-0", true, &mut cx);
    assert_eq!(
        attachment(&panes, &t, &mut cx),
        Some(Attachment::Thread(ThreadId::new("b".repeat(32))))
    );
    assert!(drawn("terminal-attachment-view-0", &mut cx).is_some());
    assert!(drawn("terminal-attachment-detach-0", &mut cx).is_some());
}

#[gpui_kit::test]
fn with_a_review_attached_and_no_prompt_reported_the_pane_offers_the_one_control_that_types_the_sentence_and_with_a_permission_prompt_or_no_review_it_does_not(
    cx: &mut TestAppContext,
) {
    let one = workspace_id('1');
    let (mut panes, mut cx, t) = one_agent(cx, SessionState::Idle, Some(Attachment::Review));
    wait_for("terminal-attachment-0", true, &mut cx);

    assert!(offers_address(&panes, &t, &mut cx));
    assert!(drawn("terminal-attachment-address-0", &mut cx).is_some());
    let before = panes.keeper.inputs(&t).len();
    click_in("terminal-attachment-address-0", &mut cx);
    assert_eq!(
        panes.keeper.input(&mut cx),
        (t.clone(), ADDRESS_EACH_COMMENT.as_bytes().to_vec())
    );
    assert_eq!(ADDRESS_EACH_COMMENT, "Address each comment.");
    assert_eq!(
        typed_since(&panes, &t, before),
        [b"Address each comment.".to_vec()]
    );
    assert_eq!(panes.focused_terminal(&mut cx), Some(t.clone()));

    for state in [SessionState::Works, SessionState::Unreviewed] {
        tell_agents(
            &panes,
            &one,
            vec![session('x', &one, &t, state, Some(Attachment::Review))],
        );
        wait_until(&mut cx, "the state", |cx| {
            panes.models.agents.read_with(cx, |agents, _| {
                agents.in_terminal(&t).is_some_and(|s| s.state == state)
            })
        });
        wait_for("terminal-attachment-address-0", true, &mut cx);
        assert!(offers_address(&panes, &t, &mut cx), "{state:?}");
    }

    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::WaitsForPermission,
            Some(Attachment::Review),
        )],
    );
    wait_for("terminal-waiting-0", true, &mut cx);
    assert!(!offers_address(&panes, &t, &mut cx));
    assert!(drawn("terminal-attachment-address-0", &mut cx).is_none());
    assert!(drawn("terminal-attachment-0", &mut cx).is_some());
    let before = panes.keeper.inputs(&t).len();
    panes.press_in_the_app::<AddressEachComment>(&mut cx);
    nothing_was_sent(&mut panes, &t, &mut cx);
    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        cx.simulate_keystrokes(key_of::<AddressEachComment>());
        nothing_was_sent(&mut panes, &t, &mut cx);
    }
    assert!(
        typed_since(&panes, &t, before)
            .concat()
            .iter()
            .all(|byte| *byte == b'z'),
        "the pane typed something for a permission prompt"
    );

    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, &t, SessionState::Idle, None)],
    );
    wait_for("terminal-attachment-0", false, &mut cx);
    assert!(!offers_address(&panes, &t, &mut cx));
    assert!(drawn("terminal-attachment-address-0", &mut cx).is_none());
    let before = panes.keeper.inputs(&t).len();
    panes.press_in_the_app::<AddressEachComment>(&mut cx);
    nothing_was_sent(&mut panes, &t, &mut cx);
    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        cx.simulate_keystrokes(key_of::<AddressEachComment>());
        nothing_was_sent(&mut panes, &t, &mut cx);
    }
    assert!(
        typed_since(&panes, &t, before)
            .concat()
            .iter()
            .all(|byte| *byte == b'z'),
        "the pane typed something with no review attached"
    );

    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::Idle,
            Some(Attachment::Review),
        )],
    );
    wait_for("terminal-attachment-address-0", true, &mut cx);
    panes.press_in_the_app::<AddressEachComment>(&mut cx);
    assert_eq!(
        panes.keeper.input(&mut cx),
        (t.clone(), b"Address each comment.".to_vec())
    );
}

#[gpui_kit::test]
fn a_session_that_ended_offers_no_sentence_even_with_a_review_attached(cx: &mut TestAppContext) {
    let (panes, mut cx, t) = one_agent(cx, SessionState::NotRunning, Some(Attachment::Review));
    wait_for("terminal-attachment-0", true, &mut cx);
    assert!(!offers_address(&panes, &t, &mut cx));
    assert!(drawn("terminal-attachment-address-0", &mut cx).is_none());
}

#[gpui_kit::test]
fn the_view_asks_again_at_each_change_of_the_reviews_of_its_workspace_and_only_while_it_is_open(
    cx: &mut TestAppContext,
) {
    let one = workspace_id('1');
    let (mut panes, mut cx, t) = one_agent(cx, SessionState::Idle, Some(Attachment::Review));
    wait_for("terminal-attachment-0", true, &mut cx);

    click_in("terminal-attachment-view-0", &mut cx);
    answer_text(&panes, &review('x'), "T1", &mut cx);
    wait_until(&mut cx, "the first text", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.viewed_attachment(&t).is_some())
    });
    assert_eq!(viewed(&panes, &t, &mut cx), Some("T1".to_owned()));

    panes.keeper.tell(Fact::ReviewsChanged {
        workspace: workspace_id('2'),
    });
    panes.keeper.tell(Fact::ReviewsChanged {
        workspace: one.clone(),
    });
    answer_text(&panes, &review('x'), "T2", &mut cx);
    wait_until(&mut cx, "the second text", |cx| {
        panes.pane.read_with(cx, |pane, _| {
            pane.viewed_attachment(&t) == Some("T2".to_owned())
        })
    });

    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::Idle,
            Some(Attachment::Thread(ThreadId::new("b".repeat(32)))),
        )],
    );
    answer_text(&panes, &review('x'), "T3", &mut cx);
    wait_until(&mut cx, "the third text", |cx| {
        panes.pane.read_with(cx, |pane, _| {
            pane.viewed_attachment(&t) == Some("T3".to_owned())
        })
    });

    panes.press_in_the_app::<ViewAttachment>(&mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);
    panes.keeper.tell(Fact::ReviewsChanged {
        workspace: one.clone(),
    });
    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(request, Request::OpenTerminal { .. }),
        "the Window asked {request:?}"
    );
    panes.keeper.reply(
        id,
        Err(common::refused("this test needs no second terminal")),
    );
}

#[gpui_kit::test]
fn the_view_closes_when_the_attachment_goes_and_when_the_keeper_has_no_text(
    cx: &mut TestAppContext,
) {
    let one = workspace_id('1');
    let (panes, mut cx, t) = one_agent(cx, SessionState::Idle, Some(Attachment::Review));
    wait_for("terminal-attachment-0", true, &mut cx);

    click_in("terminal-attachment-view-0", &mut cx);
    answer_text(&panes, &review('x'), "T", &mut cx);
    wait_for("terminal-attachment-text-0", true, &mut cx);

    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, &t, SessionState::Idle, None)],
    );
    wait_for("terminal-attachment-text-0", false, &mut cx);
    wait_for("terminal-attachment-0", false, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    tell_agents(
        &panes,
        &one,
        vec![session(
            'x',
            &one,
            &t,
            SessionState::Idle,
            Some(Attachment::Review),
        )],
    );
    wait_for("terminal-attachment-0", true, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);
    assert!(drawn("terminal-attachment-text-0", &mut cx).is_none());

    click_in("terminal-attachment-view-0", &mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(request, Request::GetAttachment { .. }));
    panes
        .keeper
        .reply(id, Err(common::refused("nothing attached")));
    // A press before the Window took the refusal closes the view itself and asks nothing. Output
    // sent after the refusal arrives after it on the link, so the press waits for that output.
    panes.keeper.output(&t, b"REFUSAL-TAKEN");
    wait_until(&mut cx, "the output after the refusal", |cx| {
        lines(&panes.pane, &t, cx)
            .iter()
            .any(|line| line.contains("REFUSAL-TAKEN"))
    });
    cx.run_until_parked();
    click_in("terminal-attachment-view-0", &mut cx);
    answer_text(&panes, &review('x'), "T", &mut cx);
    wait_for("terminal-attachment-text-0", true, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), Some("T".to_owned()));
}

#[gpui_kit::test]
fn the_keys_do_nothing_for_a_terminal_with_no_attachment_and_ask_the_keeper_nothing(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, t) = one_agent(cx, SessionState::Idle, None);
    wait_until(&mut cx, "the session", |cx| {
        panes
            .models
            .agents
            .read_with(cx, |agents, _| agents.in_terminal(&t).is_some())
    });
    panes.press_in_the_app::<ViewAttachment>(&mut cx);
    panes.press_in_the_app::<DetachAttachment>(&mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    panes.press_in_the_app::<NewTerminal>(&mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(
        matches!(request, Request::OpenTerminal { .. }),
        "the Window asked {request:?}"
    );
    panes.keeper.reply(
        id,
        Err(common::refused("this test needs no second terminal")),
    );
}

#[gpui_kit::test]
fn the_controls_of_a_slot_act_on_its_terminal_and_the_keys_on_the_focused_one(
    cx: &mut TestAppContext,
) {
    let one = workspace_id('1');
    let (mut panes, mut cx) = open_panes(cx);
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    panes.open_terminal(&t2, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        [t2.clone(), t1.clone()]
    );
    tell_agents(
        &panes,
        &one,
        vec![
            session('x', &one, &t1, SessionState::Idle, Some(Attachment::Review)),
            session('y', &one, &t2, SessionState::Idle, Some(Attachment::Review)),
        ],
    );
    wait_for("terminal-attachment-0", true, &mut cx);
    wait_for("terminal-attachment-1", true, &mut cx);

    click_in("terminal-attachment-view-1", &mut cx);
    answer_text(&panes, &review('x'), "X", &mut cx);
    wait_for("terminal-attachment-text-1", true, &mut cx);
    assert!(drawn("terminal-attachment-text-0", &mut cx).is_none());
    assert_eq!(viewed(&panes, &t1, &mut cx), Some("X".to_owned()));
    assert_eq!(viewed(&panes, &t2, &mut cx), None);
    click_in("terminal-attachment-address-1", &mut cx);
    assert_eq!(
        panes.keeper.input(&mut cx),
        (t1.clone(), b"Address each comment.".to_vec())
    );
    click_in("terminal-attachment-detach-0", &mut cx);
    answer_detach(&panes, &review('y'), &mut cx);

    click_in("terminal-attachment-detach-1", &mut cx);
    answer_detach(&panes, &review('x'), &mut cx);
    panes.click_slot(0, &mut cx);
    panes.press_in_the_app::<DetachAttachment>(&mut cx);
    answer_detach(&panes, &review('y'), &mut cx);
}

#[gpui_kit::test]
fn a_click_on_the_controls_is_not_a_click_in_the_program_and_a_click_elsewhere_in_the_slot_is(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx, t) = one_agent(cx, SessionState::Idle, Some(Attachment::Review));
    wait_for("terminal-attachment-0", true, &mut cx);

    panes
        .keeper
        .output(&t, b"\x1b[?1000h\x1b[?1006h\x1b[H\x1b[2Kmouse-on");
    wait_until(&mut cx, "the mode", |cx| {
        lines(&panes.pane, &t, cx)[0].starts_with("mouse-on")
    });

    let at = panes.cell_centre(&t, 0, 3, 2, &mut cx);
    cx.simulate_click(at, Modifiers::none());
    cx.simulate_input("z");
    let sent = panes.keeper.input_until(&t, b"z", &mut cx);
    assert!(
        sent.starts_with(b"\x1b[<0;"),
        "the program was sent {sent:?}"
    );

    click_in("terminal-attachment-address-0", &mut cx);
    assert_eq!(
        panes.keeper.input(&mut cx),
        (t.clone(), b"Address each comment.".to_vec())
    );
    click_in("terminal-attachment-view-0", &mut cx);
    answer_text(&panes, &review('x'), "T", &mut cx);
    wait_for("terminal-attachment-text-0", true, &mut cx);
    let on_text = drawn("terminal-attachment-text-0", &mut cx)
        .expect("the text is drawn")
        .center();
    cx.simulate_mouse_down(on_text, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(on_text, MouseButton::Left, Modifiers::none());
    cx.simulate_event(ScrollEvent {
        position: on_text,
        delta: ScrollDelta::Lines(point(0., 1.)),
        ..ScrollEvent::default()
    });
    cx.simulate_input("z");
    assert_eq!(panes.keeper.input_until(&t, b"z", &mut cx), b"");
}

#[gpui_kit::test]
fn a_launcher_a_pane_wants_for_a_review_is_opened_in_that_workspace_and_gets_the_focus(
    cx: &mut TestAppContext,
) {
    let one = workspace_id('1');
    let (panes, mut cx) = open_panes(cx);
    let t = terminal('c');

    panes.models.terminals.update(&mut cx, |list, cx| {
        list.want(
            Wanted::AgentFor {
                workspace: one.clone(),
                agent: "claude".to_owned(),
                review: review('r'),
            },
            cx,
        )
    });
    let (id, request) = panes.keeper.request(&mut cx);
    let Request::OpenTerminal {
        workspace,
        program,
        size: _,
    } = request
    else {
        panic!("the Window asked {request:?}");
    };
    assert_eq!(workspace, one);
    assert_eq!(
        program,
        Program::AgentFor {
            agent: "claude".to_owned(),
            review: review('r'),
        }
    );
    panes.keeper.reply(
        id,
        Ok(Reply::TerminalOpened {
            terminal: t.clone(),
        }),
    );

    wait_until(&mut cx, "the terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&t))
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        std::slice::from_ref(&t)
    );
    wait_drawn(&mut cx, "the focus in the terminal", |cx| {
        panes.focused_terminal(cx) == Some(t.clone())
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t)),
        Some(one.clone())
    );
    let held = panes
        .models
        .terminals
        .read_with(&cx, |list, _| list.terminals().to_vec());
    assert_eq!(
        held,
        [TerminalInfo {
            id: t,
            workspace: one,
            program: Program::AgentFor {
                agent: "claude".to_owned(),
                review: review('r'),
            },
        }]
    );
}

#[gpui_kit::test]
fn a_launcher_the_keeper_refuses_is_not_added_and_the_reason_is_shown(cx: &mut TestAppContext) {
    let (panes, mut cx) = open_panes(cx);
    panes.models.terminals.update(&mut cx, |list, cx| {
        list.want(
            Wanted::AgentFor {
                workspace: workspace_id('1'),
                agent: "claude".to_owned(),
                review: review('r'),
            },
            cx,
        )
    });
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(request, Request::OpenTerminal { .. }));
    panes
        .keeper
        .reply(id, Err(common::refused("no such agent today")));
    wait_until(&mut cx, "the notice", |cx| {
        panes.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, _| pane.notice())
            .expect("a notice")
            .contains("no such agent today")
    );
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, _| pane.terminals().is_empty())
    );
}
