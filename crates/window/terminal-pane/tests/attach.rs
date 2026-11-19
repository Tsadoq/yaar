//! Component test: the terminal pane attaching to terminals the Keeper holds that this
//! Window did not open, as after a plain quit and a new Window, on GPUI's test platform
//! with no window on any screen. The test plays the Keeper on the other end of a socket
//! pair: it lists terminals, answers the attach requests, and sends the kept output and the
//! live output as the Keeper does, after the reply. The pane is read back through the model
//! it shows and the elements it draws.
//!
//! A negative clause, such as "no second attach", is read after a barrier: a request the
//! test causes after it that must be the next one the Keeper hears.

mod common;

use std::time::{Duration, Instant};

use gpui_kit::{Bounds, Pixels, TestAppContext, VisualTestContext};
use yaar_link::{
    Ending, Fact, Program, Refusal, Reply, Request, RequestId, TerminalId, TerminalInfo,
    TerminalSize, WorkspaceAgents, WorkspaceId,
};
use yaar_terminal_pane::{FIRST_SIZE, NextTerminal, ScrollPageUp, ToggleSplit};

use common::{
    DEADLINE, Panes, click_in, drawn, key_of, one_repository, open_panes, open_panes_staged,
    terminal, two_workspaces, wait_until, workspace_id,
};

/// A terminal of `workspace` that runs the user's shell.
fn info(id: &TerminalId, workspace: &WorkspaceId) -> TerminalInfo {
    TerminalInfo {
        id: id.clone(),
        workspace: workspace.clone(),
        program: Program::Shell,
    }
}

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

/// The next request, which has to attach to a terminal: gives the request and what it asked.
fn attach_request(
    panes: &Panes,
    cx: &mut VisualTestContext,
) -> (RequestId, TerminalId, TerminalSize) {
    let (id, request) = panes.keeper.request(cx);
    let Request::AttachTerminal { terminal, size } = request else {
        panic!("the Window sent {request:?}, and an attach was expected");
    };
    (id, terminal, size)
}

/// Answers an attach request as the Keeper does, and sends the kept output, in as many
/// frames as it is given, and then what the program writes live.
fn answer_attach(panes: &Panes, to: RequestId, terminal: &TerminalId, cut: bool, frames: &[&[u8]]) {
    panes.keeper.reply(to, Ok(Reply::Attached { cut }));
    for frame in frames {
        panes.keeper.output(terminal, frame);
    }
}

/// A window whose Keeper holds `held` when the Window reaches it, with one workspace.
fn open_holding(cx: &mut TestAppContext, held: Vec<TerminalInfo>) -> (Panes, VisualTestContext) {
    open_panes_staged(
        cx,
        &[one_repository()],
        |keeper| keeper.set_terminals(held),
        |_, _| {},
    )
}

/// The lines of the screen of a terminal as its view reads them; none while the pane has no
/// such terminal.
fn text(panes: &Panes, id: &TerminalId, cx: &impl gpui_kit::AppContext) -> Vec<String> {
    let view = panes.pane.read_with(cx, |pane, _| pane.terminal(id));
    view.map(|view| view.read_with(cx, |view, _| view.lines()))
        .unwrap_or_default()
}

fn screen(panes: &Panes, id: &TerminalId, cx: &VisualTestContext) -> Vec<String> {
    text(panes, id, cx)
        .into_iter()
        .take_while(|line| !line.is_empty())
        .collect()
}

#[gpui_kit::test]
fn a_terminal_the_keeper_holds_at_connect_is_attached_and_shows_what_it_wrote_in_order(
    cx: &mut TestAppContext,
) {
    let t1 = terminal('a');
    let (panes, mut cx) = open_holding(cx, vec![info(&t1, &workspace_id('1'))]);

    // The size is the one the pane will show the terminal at, which it learns from the room
    // it has: not the size it opens terminals at before it was laid out.
    let (id, asked, size) = attach_request(&panes, &mut cx);
    assert_eq!(asked, t1);
    assert_ne!(size, FIRST_SIZE);

    // What the Keeper kept comes after the reply in frames, then what is written live; all
    // of it may arrive before the pane has looked, and is shown in the order it came.
    answer_attach(&panes, id, &t1, false, &[b"kept one\r\n", b"kept two\r\n"]);
    panes.keeper.output(&t1, b"live");
    wait_until(&mut cx, "the output of the terminal", |cx| {
        text(&panes, &t1, cx).first().map(String::as_str) == Some("kept one")
            && text(&panes, &t1, cx).get(2).map(String::as_str) == Some("live")
    });
    assert_eq!(screen(&panes, &t1, &cx), ["kept one", "kept two", "live"]);

    // It is a terminal of the pane like the others: listed, of its workspace, shown, not
    // cut, and the keys go to it.
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        std::slice::from_ref(&t1)
    );
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t1)),
        Some(workspace_id('1'))
    );
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        std::slice::from_ref(&t1)
    );
    assert!(!panes.pane.read_with(&cx, |pane, _| pane.attached_cut(&t1)));
    assert!(drawn("terminal-slot-0", &mut cx).is_some());
    assert!(drawn("terminal-cut-0", &mut cx).is_none());
    assert_eq!(panes.focused_terminal(&mut cx), Some(t1.clone()));
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t1.clone(), b"k".to_vec()));
    // The size came once, in the request: the view fitted the room to that very grid and
    // told the Keeper nothing more. The key is the barrier, since the resize of a view
    // that fitted another grid would have gone out before it.
    assert!(panes.keeper.resizes(&t1).is_empty());
}

#[gpui_kit::test]
fn the_output_of_a_cut_ring_starts_under_a_line_that_says_older_output_is_gone(
    cx: &mut TestAppContext,
) {
    let (cut, whole) = (terminal('a'), terminal('b'));
    let one = workspace_id('1');
    let (mut panes, mut cx) = open_holding(cx, vec![info(&cut, &one), info(&whole, &one)]);

    let (first, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, cut);
    let (second, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, whole);
    answer_attach(&panes, first, &cut, true, &[b"tail of the ring\r\n"]);
    answer_attach(&panes, second, &whole, false, &[b"all of it\r\n"]);
    wait_until(&mut cx, "both terminals", |cx| {
        screen_has(&panes, &cut, "tail of the ring", &*cx)
            && screen_has(&panes, &whole, "all of it", &*cx)
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| (
            pane.attached_cut(&cut),
            pane.attached_cut(&whole)
        )),
        (true, false)
    );
    // A terminal this Window opened was never cut.
    let opened = terminal('c');
    panes.open_terminal(&opened, &mut cx);
    assert!(
        !panes
            .pane
            .read_with(&cx, |pane, _| pane.attached_cut(&opened))
    );

    // The first terminal is shown (the pane shows one terminal of a workspace at a time:
    // the one opened after takes the slot, so the cut one is brought back with Next
    // Terminal until it is shown).
    while panes.pane.read_with(&cx, |pane, _| pane.shown()) != [cut.clone()] {
        panes.press_in_the_app::<NextTerminal>(&mut cx);
    }
    // The line says it, above the kept output, and the mark is drawn on that very line.
    let screen = text(&panes, &cut, &cx);
    assert!(
        screen[0].to_lowercase().contains("older output is gone"),
        "{screen:?}"
    );
    assert_eq!(screen[1], "tail of the ring");
    wait_drawn(&mut cx, "the cut mark", |cx| {
        drawn("terminal-cut-0", cx).is_some()
    });
    let (slot, mark) = (
        drawn("terminal-slot-0", &mut cx),
        drawn("terminal-cut-0", &mut cx),
    );
    let (slot, mark): (Bounds<Pixels>, Bounds<Pixels>) = (slot.expect("slot"), mark.expect("mark"));
    assert_eq!(mark.origin.y, slot.origin.y, "on the first line");
    assert!(mark.size.height > Pixels::ZERO && mark.size.height < slot.size.height / 4.);

    // The line is part of the output: when more output pushes it off the screen the mark
    // goes with it, and when the view is scrolled back to it the mark returns.
    let flood: Vec<u8> = (0..300)
        .flat_map(|n| format!("line {n}\r\n").into_bytes())
        .collect();
    panes.keeper.output(&cut, &flood);
    wait_drawn(&mut cx, "the mark pushed off the screen", |cx| {
        drawn("terminal-cut-0", cx).is_none()
    });
    let key = key_of::<ScrollPageUp>();
    for _ in 0..30 {
        cx.simulate_keystrokes(key);
    }
    wait_drawn(&mut cx, "the mark back in view", |cx| {
        drawn("terminal-cut-0", cx).is_some()
    });

    // The terminal that was whole carries no mark.
    while panes.pane.read_with(&cx, |pane, _| pane.shown()) != [whole.clone()] {
        panes.press_in_the_app::<NextTerminal>(&mut cx);
    }
    wait_drawn(&mut cx, "the whole terminal", |cx| {
        drawn("terminal-slot-0", cx).is_some()
    });
    assert!(drawn("terminal-cut-0", &mut cx).is_none());
}

fn screen_has(
    panes: &Panes,
    id: &TerminalId,
    wanted: &str,
    cx: &impl gpui_kit::AppContext,
) -> bool {
    text(panes, id, cx).iter().any(|line| line == wanted)
}

#[gpui_kit::test]
fn a_terminal_this_window_opened_is_listed_for_the_others_and_never_attached(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);

    // It joined the shared list, with its workspace and the shell it runs.
    let held = panes
        .models
        .terminals
        .read_with(&cx, |list, _| list.terminals().to_vec());
    assert_eq!(held, [info(&t1, &workspace_id('1'))]);

    // The barrier: the next request the Keeper hears is the one for the next terminal, so
    // no attach to the first came between.
    panes.open_terminal(&t2, &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [t1, t2]
    );
}

#[gpui_kit::test]
fn a_terminal_the_list_gains_is_attached_at_the_size_the_pane_shows_terminals_at(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = common::open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    // The pane laid its terminal out at a size of its own, which the Keeper was told: the
    // test waits until the Keeper's end has read it, since the terminal's opening names
    // another size.
    wait_until(&mut cx, "the size the pane fitted the terminal to", |_| {
        !panes.keeper.resizes(&t1).is_empty()
    });
    let shown_at = panes.grid(&t1);
    assert_ne!(shown_at, FIRST_SIZE);

    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t2, &two), cx));
    let (id, asked, size) = attach_request(&panes, &mut cx);
    assert_eq!((asked, size), (t2.clone(), shown_at));
    answer_attach(&panes, id, &t2, false, &[b"hello\r\n"]);
    wait_drawn(&mut cx, "the terminal of the second workspace", |cx| {
        drawn("terminal-slot-0", cx).is_some()
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        std::slice::from_ref(&t2)
    );

    // The size came once, in the request: the view fitted the room it has to the size it
    // was given and told the Keeper nothing more. A key typed after is the barrier.
    panes.click_slot(0, &mut cx);
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t2.clone(), b"k".to_vec()));
    assert!(panes.keeper.resizes(&t2).is_empty());
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t2)),
        Some(two)
    );
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t1)),
        Some(one)
    );
}

#[gpui_kit::test]
fn a_terminal_for_a_workspace_that_shows_none_is_attached_at_the_whole_room_in_the_split_view(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = common::open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let two = workspace_id('2');
    let (t1, t2, t3) = (terminal('a'), terminal('b'), terminal('c'));
    panes.open_terminal(&t1, &mut cx);
    wait_until(&mut cx, "the size the pane fitted the terminal to", |_| {
        !panes.keeper.resizes(&t1).is_empty()
    });
    let whole = panes.grid(&t1);

    // The split view puts every terminal of the first workspace at half the room.
    panes.open_terminal(&t2, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    wait_until(&mut cx, "the terminals at half the room", |_| {
        panes.grid(&t1).columns < whole.columns && panes.grid(&t2).columns < whole.columns
    });

    // The second workspace shows none: what the list gains for it fills the room, and
    // is asked for at that size and not at the half the last view fitted.
    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t3, &two), cx));
    let (id, asked, size) = attach_request(&panes, &mut cx);
    assert_eq!((asked, size), (t3.clone(), whole));
    answer_attach(&panes, id, &t3, false, &[b"hello\r\n"]);
    wait_drawn(&mut cx, "the terminal of the second workspace", |cx| {
        drawn("terminal-slot-0", cx).is_some()
    });
    panes.click_slot(0, &mut cx);
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t3.clone(), b"k".to_vec()));
    assert!(panes.keeper.resizes(&t3).is_empty());
}

#[gpui_kit::test]
fn a_terminal_gained_for_a_split_workspace_is_attached_at_the_size_of_a_slot(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = workspace_id('1');
    let (t1, t2, t3) = (terminal('a'), terminal('b'), terminal('c'));
    panes.open_terminal(&t1, &mut cx);
    wait_until(&mut cx, "the size the pane fitted the terminal to", |_| {
        !panes.keeper.resizes(&t1).is_empty()
    });
    let whole = panes.grid(&t1);

    // The workspace is split: its terminals are at half the room.
    panes.open_terminal(&t2, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    wait_until(&mut cx, "the terminals at half the room", |_| {
        panes.grid(&t1).columns < whole.columns && panes.grid(&t2).columns < whole.columns
    });
    let half = panes.grid(&t1);

    // What the list gains for it is shown in a slot when the user asks for it, so it is
    // asked for at the size of a slot and not at the whole room.
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t3, &one), cx));
    let (id, asked, size) = attach_request(&panes, &mut cx);
    assert_eq!((asked, size), (t3.clone(), half));
    answer_attach(&panes, id, &t3, false, &[b"hello\r\n"]);
    wait_until(&mut cx, "the attached terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&t3))
    });
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    wait_drawn(&mut cx, "the attached terminal in a slot", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.shown().contains(&t3))
    });
    // The size came once, in the request: a key typed after is the barrier, since the
    // resize of a view that fitted another grid would have gone out before it.
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t3.clone(), b"k".to_vec()));
    assert!(panes.keeper.resizes(&t3).is_empty());
}

#[gpui_kit::test]
fn a_terminal_gained_for_a_workspace_that_shows_one_is_attached_at_the_whole_room_whatever_another_workspace_fitted(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = common::open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2, t3, t4) = (terminal('a'), terminal('b'), terminal('c'), terminal('d'));
    panes.open_terminal(&t1, &mut cx);
    wait_until(&mut cx, "the size the pane fitted the terminal to", |_| {
        !panes.keeper.resizes(&t1).is_empty()
    });
    let whole = panes.grid(&t1);

    // The second workspace is split, and its terminals are the last the pane fitted.
    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    panes.open_terminal(&t2, &mut cx);
    panes.open_terminal(&t3, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    wait_until(&mut cx, "the terminals at half the room", |_| {
        panes.grid(&t2).columns < whole.columns && panes.grid(&t3).columns < whole.columns
    });

    // What the list gains for the first workspace, which shows one terminal, is asked for
    // at the whole room that terminal will be shown at, and not at the half the last view
    // fitted.
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t4, &one), cx));
    let (id, asked, size) = attach_request(&panes, &mut cx);
    assert_eq!((asked, size), (t4.clone(), whole));
    answer_attach(&panes, id, &t4, false, &[b"hello\r\n"]);
    wait_until(&mut cx, "the attached terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&t4))
    });
    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    wait_drawn(&mut cx, "the attached terminal in its slot", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.shown() == std::slice::from_ref(&t4))
    });
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t4.clone(), b"k".to_vec()));
    assert!(panes.keeper.resizes(&t4).is_empty());
}

#[gpui_kit::test]
fn a_terminal_gained_for_a_split_workspace_is_attached_at_a_slot_whatever_another_workspace_fitted(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = common::open_panes_listing(cx, &[two_workspaces()], |_, _| {});
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let (t1, t2, t3, t4) = (terminal('a'), terminal('b'), terminal('c'), terminal('d'));

    // The first workspace is split: its two terminals are at half the room.
    panes.open_terminal(&t1, &mut cx);
    wait_until(&mut cx, "the size the pane fitted the terminal to", |_| {
        !panes.keeper.resizes(&t1).is_empty()
    });
    let whole = panes.grid(&t1);
    panes.open_terminal(&t2, &mut cx);
    panes.press_in_the_app::<ToggleSplit>(&mut cx);
    wait_until(&mut cx, "the terminals at half the room", |_| {
        panes.grid(&t1).columns < whole.columns && panes.grid(&t2).columns < whole.columns
    });
    let half = panes.grid(&t1);

    // The second workspace shows one terminal, which fills the room: that is the last
    // size the pane fitted.
    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    panes.open_terminal(&t3, &mut cx);
    wait_until(&mut cx, "the terminal alone at the whole room", |_| {
        panes.grid(&t3) == whole
    });

    // What the list gains for the first workspace is shown in a slot, so it is asked for
    // at the size of a slot and not at the whole room the other workspace fitted last.
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t4, &one), cx));
    let (id, asked, size) = attach_request(&panes, &mut cx);
    assert_eq!((asked, size), (t4.clone(), half));
    answer_attach(&panes, id, &t4, false, &[b"hello\r\n"]);
    wait_until(&mut cx, "the attached terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&t4))
    });
    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    wait_drawn(&mut cx, "the attached terminal in a slot", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.shown().contains(&t4))
    });
    // The size came once, in the request: a key typed after is the barrier, since the
    // resize of a view that fitted another grid would have gone out before it.
    cx.simulate_input("k");
    assert_eq!(panes.keeper.input(&mut cx), (t4.clone(), b"k".to_vec()));
    assert!(panes.keeper.resizes(&t4).is_empty());
}

#[gpui_kit::test]
fn a_terminal_being_attached_is_not_attached_again_when_the_list_changes(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = workspace_id('1');
    let (t1, t2, t3, t4) = (terminal('a'), terminal('b'), terminal('c'), terminal('d'));
    panes.open_terminal(&t1, &mut cx);

    // Two terminals join the list one after the other, the first still unanswered.
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t2, &one), cx));
    let (first, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, t2);
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t3, &one), cx));
    let (second, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, t3);
    answer_attach(&panes, first, &t2, false, &[]);
    answer_attach(&panes, second, &t3, false, &[]);
    wait_until(&mut cx, "both attached", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().len() == 3)
    });

    // The barrier: the next request is the one for a new terminal, and the list, changed
    // by that terminal's joining it, has not sent another attach.
    panes.open_terminal(&t4, &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [t1, t2, t3, t4]
    );
}

#[gpui_kit::test]
fn an_attach_the_keeper_refuses_adds_no_terminal(cx: &mut TestAppContext) {
    let (gone, next) = (terminal('a'), terminal('b'));
    let (mut panes, mut cx) = open_holding(cx, vec![info(&gone, &workspace_id('1'))]);
    let (id, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, gone);
    panes
        .keeper
        .reply(id, Err(Refusal::UnknownTerminal { terminal: gone }));

    // The pane goes on as before: the next request it makes is the one the user asked
    // for, and the terminal it opens is its only one.
    panes.open_terminal(&next, &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [next]
    );
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.notice()), None);
}

#[gpui_kit::test]
fn a_terminal_of_a_workspace_off_screen_is_shown_when_its_workspace_comes_on_screen(
    cx: &mut TestAppContext,
) {
    let (one, two) = (workspace_id('1'), workspace_id('2'));
    let t1 = terminal('a');
    let (panes, mut cx) = open_panes_staged(
        cx,
        &[two_workspaces()],
        |keeper| keeper.set_terminals(vec![info(&t1, &workspace_id('2'))]),
        |_, _| {},
    );
    let (id, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, t1);
    answer_attach(&panes, id, &t1, false, &[b"over there\r\n"]);
    wait_until(&mut cx, "the terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().contains(&t1))
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.workspace(&t1)),
        Some(two.clone())
    );
    // The workspace on screen is the first, which has none.
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.shown()), []);
    assert!(drawn("terminal-slot-0", &mut cx).is_none());

    click_in(&format!("sidebar-workspace-{two}"), &mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        std::slice::from_ref(&t1)
    );
    wait_until(&mut cx, "its output", |cx| {
        screen_has(&panes, &t1, "over there", &*cx)
    });
    click_in(&format!("sidebar-workspace-{one}"), &mut cx);
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.shown()), []);
}

#[gpui_kit::test]
fn an_attached_terminal_whose_program_ends_says_how(cx: &mut TestAppContext) {
    let t1 = terminal('a');
    let (panes, mut cx) = open_holding(cx, vec![info(&t1, &workspace_id('1'))]);
    let (id, _, _) = attach_request(&panes, &mut cx);
    answer_attach(&panes, id, &t1, false, &[b"bye\r\n"]);
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: t1.clone(),
        ending: Ending::Exited(0),
    });
    wait_until(&mut cx, "the end", |cx| {
        let view = panes.pane.read_with(cx, |pane, _| pane.terminal(&t1));
        view.is_some_and(|view| {
            view.read_with(cx, |view, _| view.ending()) == Some(Ending::Exited(0))
        })
    });
    assert!(screen_has(&panes, &t1, "bye", &cx));
}

#[gpui_kit::test]
fn an_ending_told_before_the_pane_takes_the_answer_to_its_attach_reaches_its_view(
    cx: &mut TestAppContext,
) {
    let (one, t1) = (workspace_id('1'), terminal('a'));
    let (panes, mut cx) = open_holding(cx, vec![info(&t1, &one)]);
    let (id, _, _) = attach_request(&panes, &mut cx);

    // The program writes and ends while the answer to the attach is not yet taken. The fact
    // after the ending is the barrier: facts are told in the order they came, so when the
    // agents of the workspace are held, the pane has heard the ending.
    panes.keeper.output(&t1, b"bye\r\n");
    panes.keeper.tell(Fact::TerminalEnded {
        terminal: t1.clone(),
        ending: Ending::Exited(3),
    });
    panes.keeper.tell(Fact::AgentsChanged(WorkspaceAgents {
        workspace: one.clone(),
        shown: None,
        sessions: Vec::new(),
    }));
    let models = panes.models.clone();
    wait_until(&mut cx, "the agents of the workspace", move |cx| {
        models
            .agents
            .read_with(cx, |agents, _| agents.of(&one).is_some())
    });
    assert!(
        panes
            .pane
            .read_with(&cx, |pane, _| pane.terminals())
            .is_empty()
    );

    // The answer comes after the ending: the view the pane makes then says how it ended.
    answer_attach(&panes, id, &t1, false, &[]);
    wait_until(&mut cx, "the ending in the view", |cx| {
        let view = panes.pane.read_with(cx, |pane, _| pane.terminal(&t1));
        view.is_some_and(|view| {
            view.read_with(cx, |view, _| view.ending()) == Some(Ending::Exited(3))
        })
    });
    assert!(screen_has(&panes, &t1, "bye", &cx));
}

#[gpui_kit::test]
fn a_terminal_attached_in_a_workspace_that_shows_one_takes_no_slot_from_it(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let one = workspace_id('1');
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    assert_eq!(panes.focused_terminal(&mut cx), Some(t1.clone()));

    // The list gains a terminal of the workspace on screen, which the user looks at one
    // terminal of.
    panes
        .models
        .terminals
        .update(&mut cx, |list, cx| list.opened(info(&t2, &one), cx));
    let (id, asked, _) = attach_request(&panes, &mut cx);
    assert_eq!(asked, t2);
    answer_attach(&panes, id, &t2, false, &[b"hello\r\n"]);
    wait_until(&mut cx, "the attached terminal in the pane", |cx| {
        panes
            .pane
            .read_with(cx, |pane, _| pane.terminals().len() == 2)
    });

    // It is listed, and the user's terminal is still the one shown and focused.
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.terminals()),
        [t1.clone(), t2.clone()]
    );
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        std::slice::from_ref(&t1)
    );
    assert_eq!(panes.focused_terminal(&mut cx), Some(t1.clone()));

    // And it is reached with Next Terminal.
    panes.press_in_the_app::<NextTerminal>(&mut cx);
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.shown()),
        std::slice::from_ref(&t2)
    );
}
