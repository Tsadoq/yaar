//! Component test: the review attached to an agent's next message, shown at the agent's
//! terminal, on GPUI's test platform with no window on any screen. The test plays the
//! Keeper on the other end of a socket pair: it tells which sessions run in which
//! terminals, what is attached to them and what they do, and answers the questions the
//! pane asks. The pane is read back through the model it shows, the elements it draws and
//! the bytes it sends to a program.
//!
//! A negative clause is read after a barrier: something the test causes after it, through
//! the same path, whose arrival shows that everything before it has arrived. For a key
//! that sends nothing the barrier is a letter typed into the terminal afterwards, which
//! travels the path the sentence would; for a request that is not sent, the next request
//! the test causes.

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

/// The review of the session `x`.
fn review(name: char) -> ReviewId {
    ReviewId::new(name.to_string().repeat(32))
}

/// A session of Claude Code in `workspace`, in `terminal`, that reviews `review`, with
/// `attached` going with its next message.
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

/// Tells the Window the sessions of one workspace, as the Keeper does when they change.
fn tell_agents(panes: &Panes, workspace: &WorkspaceId, sessions: Vec<Session>) {
    let shown = sessions.iter().map(|session| session.state).min();
    panes.keeper.tell(Fact::AgentsChanged(WorkspaceAgents {
        workspace: workspace.clone(),
        shown,
        sessions,
    }));
}

/// Waits until a selector is drawn, or is not.
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

/// The next request, which has to be the question for the text of `review`; answers it
/// with `text`.
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

/// The next request, which has to be the detach of `review`; answers it.
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

/// Shows that what the test did since the last look sent the program of `id` nothing: a
/// letter is typed into the terminal, and what arrived before it is read.
fn nothing_was_sent(panes: &mut Panes, id: &TerminalId, cx: &mut VisualTestContext) {
    panes.click_slot(0, cx);
    cx.simulate_input("z");
    assert_eq!(panes.keeper.input_until(id, b"z", cx), b"");
}

/// Shows that no request is left unread: a new terminal is asked for, and the next request
/// has to be that one. A surplus request sent before it would be read in its place.
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

/// A pane with one terminal `t` in slot 0 that has the focus, in the workspace on
/// screen, and the session `x` that runs in it, in `state`, with `attached`.
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

/// The frames of input the pane sent `id` after the first `before` ones.
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

    // Nothing is attached: no line. The mark of a session that waits, told after, is the
    // barrier: it arrives through the agents the line is read from.
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

    // A review is attached to the next message of the session, which is idle.
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

    // The control that views it asks for what will be sent and shows the answer.
    click_in("terminal-attachment-view-0", &mut cx);
    answer_text(&panes, &review('x'), "T", &mut cx);
    wait_until(&mut cx, "the text of the attachment", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.viewed_attachment(&t).is_some())
    });
    assert_eq!(viewed(&panes, &t, &mut cx), Some("T".to_owned()));
    wait_for("terminal-attachment-text-0", true, &mut cx);

    // Pressed again, it hides the text.
    click_in("terminal-attachment-view-0", &mut cx);
    wait_for("terminal-attachment-text-0", false, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    // The key does what the control does, from outside the terminal.
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

    // The control that detaches it sends the detach of the review. No question for the
    // text was sent in between: this is the next request.
    // One press sends one: what follows each is the next request, not a second detach.
    click_in("terminal-attachment-detach-0", &mut cx);
    answer_detach(&panes, &review('x'), &mut cx);
    no_other_request_was_sent(&mut panes, &mut cx);
    // Its key does the same, from outside the terminal.
    panes.press_in_the_app::<DetachAttachment>(&mut cx);
    answer_detach(&panes, &review('x'), &mut cx);
    no_other_request_was_sent(&mut panes, &mut cx);
    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        cx.simulate_keystrokes(key_of::<DetachAttachment>());
        answer_detach(&panes, &review('x'), &mut cx);
        no_other_request_was_sent(&mut panes, &mut cx);
    }

    // The Keeper tells it is detached: the line is gone.
    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, &t, SessionState::Idle, None)],
    );
    wait_for("terminal-attachment-0", false, &mut cx);
    assert_eq!(attachment(&panes, &t, &mut cx), None);
    assert!(drawn("terminal-attachment-view-0", &mut cx).is_none());
    assert!(drawn("terminal-attachment-detach-0", &mut cx).is_none());

    // One thread attached alone is shown as well.
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

    // Attached and no prompt reported: offered. A click types the sentence, which is the
    // only thing the pane sent the program: not a line end, not a paste.
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
    // The user presses Enter in the terminal, which has the focus again.
    assert_eq!(panes.focused_terminal(&mut cx), Some(t.clone()));

    // While the agent works, a review can be attached and the control is there too, and
    // while it waits for the next message with changes unreviewed.
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

    // A permission prompt reported: the review stays attached and shown, and the control
    // is not offered. Its key sends nothing.
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

    // No review attached: not offered either, and the key sends nothing.
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

    // The key types the sentence when it is offered, from outside the terminal.
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

    // Open, a change of another workspace's reviews asks nothing, and one of its own does.
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

    // The attachment changes from the review to one thread: the text is asked again.
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

    // Closed again, a change asks nothing: the question that follows is for the next view.
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

    // The review is detached: the line and the text go.
    tell_agents(
        &panes,
        &one,
        vec![session('x', &one, &t, SessionState::Idle, None)],
    );
    wait_for("terminal-attachment-text-0", false, &mut cx);
    wait_for("terminal-attachment-0", false, &mut cx);
    assert_eq!(viewed(&panes, &t, &mut cx), None);

    // Attached again, the view is closed: it does not come back by itself.
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

    // A Keeper that has no text to give leaves the view closed, and the next press asks
    // again instead of closing it.
    click_in("terminal-attachment-view-0", &mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert!(matches!(request, Request::GetAttachment { .. }));
    panes
        .keeper
        .reply(id, Err(common::refused("nothing attached")));
    // The refusal closes the view, and a press that comes before the Window took it
    // closes the view itself, which asks nothing. Output the Keeper sends after the
    // refusal comes after it on the link, so the press waits until that output is on the
    // screen.
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

    // The next request the Window sends is the one that follows: neither key sent one.
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

    // Slot 1 shows `t1`, the session `x`: its controls are its own.
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

    // The keys go to the terminal that has the focus: the click on the control gave
    // it to the slot of the control.
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

    // The program asks for the mouse; its first line says it did.
    panes
        .keeper
        .output(&t, b"\x1b[?1000h\x1b[?1006h\x1b[H\x1b[2Kmouse-on");
    wait_until(&mut cx, "the mode", |cx| {
        lines(&panes.pane, &t, cx)[0].starts_with("mouse-on")
    });

    // A click in the program's own area is reported to it.
    let at = panes.cell_centre(&t, 0, 3, 2, &mut cx);
    cx.simulate_click(at, Modifiers::none());
    cx.simulate_input("z");
    let sent = panes.keeper.input_until(&t, b"z", &mut cx);
    assert!(
        sent.starts_with(b"\x1b[<0;"),
        "the program was sent {sent:?}"
    );

    // A click on a control is not: what the program receives is the sentence, and then
    // nothing of what follows on the controls and on the text that lies over it.
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
    // The focus is in the terminal still, and a click in it would be reported: the letter
    // is typed where it is.
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

    // The review pane, or any pane, wants the launcher of Claude Code for a review.
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

    // The terminal is in the pane, shown, with the focus, as a terminal opened with New
    // Terminal is.
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
    // It is listed for every pane as the launcher it is, not as a shell.
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
