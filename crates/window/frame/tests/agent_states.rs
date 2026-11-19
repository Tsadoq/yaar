//! Component test: what the Frame shows of the agents (the state of each workspace in the
//! sidebar and the notice of a turn that ended where the window did not look) and the two
//! ways to quit. Keys and clicks go in the way a user's do on GPUI's test platform; what is
//! shown is read back through the models it draws from. The test plays the Keeper on the
//! other end of a socket pair.
//!
//! A negative clause ("no notice", "no stop frame") is read only after a barrier: a fact
//! told, or a request sent, through the same path after the thing that must not have
//! happened, whose effect shows that everything before it has arrived.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    Action, KeyContext, Keystroke, Modifiers, Pixels, Size, TestAppContext, VisualTestContext, px,
    size,
};
use yaar_frame::{
    KEY_CONTEXT, Quit, QuitAndStopAgents, RAIL_BELOW, RAIL_WIDTH, Sends, ShowFinishedTurn,
    SidebarRow, controls,
};
use yaar_link::{
    Fact, Hello, PROTOCOL_VERSION, Session, SessionId, SessionState, TurnId, WorkspaceAgents,
    WorkspaceId,
};

use common::{
    Shown, drawn, id, open_frame, open_frame_staged, open_frame_with_keeper, pressed, repository,
    wait_until, workspace,
};

/// The size the app's window opens at: wide enough for the full list.
const WIDE: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

/// The four states the sidebar marks, with the word that ends their selector.
const MARKED: [(SessionState, &str); 4] = [
    (SessionState::Works, "working"),
    (SessionState::WaitsForPermission, "waiting"),
    (SessionState::Unreviewed, "unreviewed"),
    (SessionState::NotRunning, "not-running"),
];

/// A session of Claude Code, named by a letter, in workspace `workspace`.
fn session(name: char, workspace: char, state: SessionState) -> Session {
    Session {
        id: SessionId::new(name.to_string().repeat(32)),
        agent: "claude".to_owned(),
        title: format!("Claude Code {name}"),
        workspace: id(workspace),
        terminal: None,
        state,
        turn: None,
        review: None,
    }
}

/// What the Keeper tells of a workspace with one session in this state.
fn agents_of(workspace: char, state: SessionState) -> WorkspaceAgents {
    WorkspaceAgents {
        workspace: id(workspace),
        shown: Some(state),
        sessions: vec![session(workspace, workspace, state)],
    }
}

/// The turn that ends in a fact: its id is a letter repeated.
fn turn(name: char) -> TurnId {
    TurnId::new(name.to_string().repeat(32))
}

/// A turn of the session `session_in` that ended in workspace `workspace`.
fn turn_ended(workspace: char, session_in: char, turn_name: char) -> Fact {
    Fact::TurnEnded {
        workspace: id(workspace),
        session: session(session_in, workspace, SessionState::Unreviewed).id,
        turn: turn(turn_name),
        state: None,
        since: None,
    }
}

/// Five worktrees of one repository; the first is on screen once the Keeper lists them.
fn repositories() -> Vec<yaar_link::Repository> {
    vec![repository(
        'a',
        vec![
            workspace('1', "main"),
            workspace('2', "feature"),
            workspace('3', "fix"),
            workspace('4', "docs"),
            workspace('5', "chore"),
        ],
    )]
}

/// A window whose Keeper has listed the five workspaces, with the first on screen.
fn showing(cx: &mut TestAppContext) -> (Shown, VisualTestContext) {
    let (shown, cx) = open_frame(WIDE, Vec::new(), cx);
    listed(shown, cx)
}

fn listed(mut shown: Shown, mut cx: VisualTestContext) -> (Shown, VisualTestContext) {
    shown.keeper.list(&repositories(), &mut cx);
    wait_until(&mut cx, "the five workspaces in the sidebar", |cx| {
        rows(&shown, cx).len() == 6
    });
    (shown, cx)
}

fn rows(shown: &Shown, cx: &mut TestAppContext) -> Vec<SidebarRow> {
    shown.sidebar.read_with(cx, |sidebar, cx| sidebar.rows(cx))
}

/// The state the sidebar's row of a workspace reads.
fn agent_of(shown: &Shown, workspace: char, cx: &mut TestAppContext) -> Option<SessionState> {
    rows(shown, cx)
        .into_iter()
        .find_map(|row| match row {
            SidebarRow::Workspace {
                id: found, agent, ..
            } if found == id(workspace) => Some(agent),
            _ => None,
        })
        .expect("the workspace has a row")
}

/// The words of the marks drawn in the row of a workspace, whichever they are.
fn marks_drawn(workspace: char, cx: &mut VisualTestContext) -> Vec<&'static str> {
    MARKED
        .iter()
        .filter(|(_, word)| drawn(&format!("sidebar-agent-{}-{word}", id(workspace)), cx).is_some())
        .map(|(_, word)| *word)
        .collect()
}

/// Holds that exactly the mark of `word` is drawn for the workspace, inside its row.
fn assert_marked(workspace: char, word: &str, cx: &mut VisualTestContext) {
    assert_eq!(
        marks_drawn(workspace, cx),
        [MARKED.iter().find(|(_, w)| *w == word).expect("a word").1],
        "the marks of workspace {workspace}"
    );
    let row = drawn(&format!("sidebar-workspace-{}", id(workspace)), cx).expect("the row");
    let mark = drawn(&format!("sidebar-agent-{}-{word}", id(workspace)), cx).expect("the mark");
    assert!(
        mark.size.width > px(0.) && mark.size.height > px(0.),
        "the mark of workspace {workspace} is drawn with no room: {mark:?}"
    );
    assert!(
        row.left() <= mark.left()
            && mark.right() <= row.right()
            && row.top() <= mark.top()
            && mark.bottom() <= row.bottom(),
        "the mark {mark:?} lies outside its row {row:?}"
    );
}

/// Clicks the row of a workspace where it was drawn.
fn click_workspace(workspace: char, cx: &mut VisualTestContext) {
    let row = drawn(&format!("sidebar-workspace-{}", id(workspace)), cx).expect("the row");
    cx.simulate_click(row.center(), Modifiers::none());
}

fn on_screen(shown: &Shown, cx: &mut TestAppContext) -> Option<WorkspaceId> {
    shown
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
}

fn notice_text(shown: &Shown, cx: &mut TestAppContext) -> Option<String> {
    shown
        .frame
        .read_with(cx, |frame, cx| frame.turn_notice(cx))
        .map(|notice| notice.text)
}

#[gpui_kit::test]
fn the_sidebar_shows_for_each_workspace_whether_its_agent_works_waits_for_permission_has_finished_a_turn_not_yet_reviewed_or_is_not_running(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(cx);

    // One workspace in each of the four states, and a fifth no agent ever reported in.
    for (workspace, (state, _)) in ['1', '2', '3', '4'].into_iter().zip(MARKED) {
        shown
            .keeper
            .tell(Fact::AgentsChanged(agents_of(workspace, state)));
    }
    wait_until(&mut cx, "the four states in the sidebar", |cx| {
        ['1', '2', '3', '4']
            .into_iter()
            .all(|workspace| agent_of(&shown, workspace, cx).is_some())
    });
    for (workspace, (state, word)) in ['1', '2', '3', '4'].into_iter().zip(MARKED) {
        assert_eq!(agent_of(&shown, workspace, &mut cx), Some(state));
        assert_marked(workspace, word, &mut cx);
    }
    assert_eq!(agent_of(&shown, '5', &mut cx), None);
    assert_eq!(marks_drawn('5', &mut cx), Vec::<&str>::new());

    // A later fact changes one workspace. The click on another row puts that one on
    // screen, so the sidebar is drawn from the model and not from what was selected.
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '1',
        SessionState::Unreviewed,
    )));
    wait_until(&mut cx, "the new state of workspace 1", |cx| {
        agent_of(&shown, '1', cx) == Some(SessionState::Unreviewed)
    });
    click_workspace('2', &mut cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('2')));
    assert_eq!(
        agent_of(&shown, '1', &mut cx),
        Some(SessionState::Unreviewed)
    );
    assert_marked('1', "unreviewed", &mut cx);
    assert_eq!(
        agent_of(&shown, '2', &mut cx),
        Some(SessionState::WaitsForPermission)
    );
    assert_marked('2', "waiting", &mut cx);
    assert_eq!(
        agent_of(&shown, '3', &mut cx),
        Some(SessionState::Unreviewed)
    );
    assert_marked('3', "unreviewed", &mut cx);
    assert_eq!(
        agent_of(&shown, '4', &mut cx),
        Some(SessionState::NotRunning)
    );
    assert_marked('4', "not-running", &mut cx);
    assert_eq!(agent_of(&shown, '5', &mut cx), None);
}

#[gpui_kit::test]
fn the_states_the_keeper_lists_when_the_window_reaches_it_are_shown_with_no_fact_told(
    cx: &mut TestAppContext,
) {
    // A Window opened after a plain quit: the agents were there before it.
    let keeper = Hello {
        protocol: PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    };
    let (shown, cx) = open_frame_staged(keeper, WIDE, Vec::new(), cx, |keeper| {
        keeper.set_agents(vec![
            agents_of('2', SessionState::Works),
            agents_of('4', SessionState::Unreviewed),
        ]);
    });
    let (shown, mut cx) = listed(shown, cx);

    wait_until(&mut cx, "the listed states in the sidebar", |cx| {
        agent_of(&shown, '4', cx) == Some(SessionState::Unreviewed)
    });
    assert_eq!(agent_of(&shown, '2', &mut cx), Some(SessionState::Works));
    assert_marked('2', "working", &mut cx);
    assert_marked('4', "unreviewed", &mut cx);
    assert_eq!(marks_drawn('1', &mut cx), Vec::<&str>::new());
}

#[gpui_kit::test]
fn an_agent_that_runs_with_nothing_to_review_is_listed_idle_and_draws_no_mark(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(cx);
    shown
        .keeper
        .tell(Fact::AgentsChanged(agents_of('3', SessionState::Idle)));
    wait_until(&mut cx, "the idle state", |cx| {
        agent_of(&shown, '3', cx) == Some(SessionState::Idle)
    });
    assert_eq!(marks_drawn('3', &mut cx), Vec::<&str>::new());

    // The same workspace, once the agent works: the mark follows the state at once.
    shown
        .keeper
        .tell(Fact::AgentsChanged(agents_of('3', SessionState::Works)));
    wait_until(&mut cx, "the working state", |cx| {
        agent_of(&shown, '3', cx) == Some(SessionState::Works)
    });
    assert_marked('3', "working", &mut cx);
}

#[gpui_kit::test]
fn the_rail_draws_the_marks_too_inside_the_rail(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = showing(cx);
    for (workspace, (state, _)) in ['1', '2', '3', '4'].into_iter().zip(MARKED) {
        shown
            .keeper
            .tell(Fact::AgentsChanged(agents_of(workspace, state)));
    }
    wait_until(&mut cx, "the states", |cx| {
        agent_of(&shown, '4', cx).is_some()
    });

    cx.simulate_resize(size(RAIL_BELOW - px(1.), px(800.)));
    for (workspace, (_, word)) in ['1', '2', '3', '4'].into_iter().zip(MARKED) {
        assert_marked(workspace, word, &mut cx);
        let mark =
            drawn(&format!("sidebar-agent-{}-{word}", id(workspace)), &mut cx).expect("the mark");
        assert!(
            mark.right() <= RAIL_WIDTH,
            "the mark of workspace {workspace} ends at {:?}, outside the rail",
            mark.right()
        );
    }
    assert_eq!(marks_drawn('5', &mut cx), Vec::<&str>::new());
}

#[gpui_kit::test]
fn a_turn_that_ends_in_a_workspace_not_on_screen_changes_its_state_in_the_sidebar_and_shows_a_notice_whose_control_switches_to_it(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));
    assert_eq!(notice_text(&shown, &mut cx), None);

    // The turn of workspace 2 ends: the Keeper tells the new state, then the end.
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '2',
        SessionState::Unreviewed,
    )));
    shown.keeper.tell(turn_ended('2', '2', 'a'));
    wait_until(&mut cx, "the notice", |cx| {
        notice_text(&shown, cx).is_some()
    });

    // Its state in the sidebar.
    assert_eq!(
        agent_of(&shown, '2', &mut cx),
        Some(SessionState::Unreviewed)
    );
    assert_marked('2', "unreviewed", &mut cx);
    // The notice: it names the session and the workspace, and is drawn with its control.
    let notice = shown
        .frame
        .read_with(&cx, |frame, cx| frame.turn_notice(cx))
        .expect("a notice");
    assert_eq!(notice.workspace, id('2'));
    assert_eq!(notice.session, session('2', '2', SessionState::Works).id);
    assert_eq!(notice.text, "Claude Code 2 finished a turn in feature");
    let drawn_notice = drawn("frame-turn-notice", &mut cx).expect("the notice is drawn");
    let control = drawn("frame-turn-notice-show", &mut cx).expect("its control is drawn");
    assert!(
        drawn_notice.left() <= control.left()
            && control.right() <= drawn_notice.right()
            && drawn_notice.top() <= control.top()
            && control.bottom() <= drawn_notice.bottom(),
        "the control {control:?} lies outside the notice {drawn_notice:?}"
    );
    // Nothing moved by itself: the workspace on screen is still the first.
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));

    // The control puts the workspace of the notice on screen, and the notice goes.
    cx.simulate_click(control.center(), Modifiers::none());
    assert_eq!(on_screen(&shown, &mut cx), Some(id('2')));
    assert_eq!(notice_text(&shown, &mut cx), None);
    assert_eq!(drawn("frame-turn-notice", &mut cx), None);
    assert_eq!(drawn("frame-turn-notice-show", &mut cx), None);
    wait_until(&mut cx, "the Keeper told of workspace 2", |_| {
        shown.keeper.shown_workspace() == Some(id('2'))
    });

    // Again, with the key of the table: back to workspace 1, a new turn ends in 3.
    click_workspace('1', &mut cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '3',
        SessionState::Unreviewed,
    )));
    shown.keeper.tell(turn_ended('3', '3', 'b'));
    wait_until(&mut cx, "the second notice", |cx| {
        notice_text(&shown, cx).is_some()
    });
    assert_eq!(
        notice_text(&shown, &mut cx).as_deref(),
        Some("Claude Code 3 finished a turn in fix")
    );
    cx.simulate_keystrokes(&pressed::<ShowFinishedTurn>());
    assert_eq!(on_screen(&shown, &mut cx), Some(id('3')));
    assert_eq!(notice_text(&shown, &mut cx), None);
    assert_eq!(drawn("frame-turn-notice", &mut cx), None);

    // A turn that ends in the workspace on screen makes no notice. The fact that follows
    // it, a state not told before, is the barrier: facts are read in the order they were
    // told, so once the row shows that state the end has been taken in.
    shown.keeper.tell(turn_ended('3', '3', 'c'));
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '3',
        SessionState::NotRunning,
    )));
    wait_until(&mut cx, "the state after the barrier", |cx| {
        agent_of(&shown, '3', cx) == Some(SessionState::NotRunning)
    });
    assert_eq!(notice_text(&shown, &mut cx), None);
    assert_eq!(drawn("frame-turn-notice", &mut cx), None);
    // Nor one that waits for the user to leave: the end was not recorded for later.
    click_workspace('1', &mut cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));
    assert_eq!(notice_text(&shown, &mut cx), None);
    assert_eq!(drawn("frame-turn-notice", &mut cx), None);
    assert_eq!(drawn("frame-turn-notice-show", &mut cx), None);
}

#[gpui_kit::test]
fn of_two_turns_that_ended_elsewhere_the_later_one_is_shown_and_one_in_the_workspace_on_screen_leaves_it(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(cx);
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '2',
        SessionState::Unreviewed,
    )));
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '3',
        SessionState::Unreviewed,
    )));
    shown.keeper.tell(turn_ended('2', '2', 'a'));
    shown.keeper.tell(turn_ended('3', '3', 'b'));
    // The barrier is a later state of workspace 3.
    shown
        .keeper
        .tell(Fact::AgentsChanged(agents_of('3', SessionState::Works)));
    wait_until(&mut cx, "the state after the barrier", |cx| {
        agent_of(&shown, '3', cx) == Some(SessionState::Works)
    });
    assert_eq!(
        notice_text(&shown, &mut cx).as_deref(),
        Some("Claude Code 3 finished a turn in fix")
    );

    // A turn that ends in the workspace on screen leaves the notice as it was.
    shown.keeper.tell(turn_ended('1', '1', 'c'));
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '1',
        SessionState::Unreviewed,
    )));
    wait_until(&mut cx, "the state of workspace 1", |cx| {
        agent_of(&shown, '1', cx) == Some(SessionState::Unreviewed)
    });
    assert_eq!(
        notice_text(&shown, &mut cx).as_deref(),
        Some("Claude Code 3 finished a turn in fix")
    );
}

#[gpui_kit::test]
fn a_notice_goes_when_its_workspace_comes_on_screen_by_any_means_and_does_not_come_back_when_it_leaves(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(cx);
    shown.keeper.tell(Fact::AgentsChanged(agents_of(
        '2',
        SessionState::Unreviewed,
    )));
    shown.keeper.tell(turn_ended('2', '2', 'a'));
    wait_until(&mut cx, "the notice", |cx| {
        notice_text(&shown, cx).is_some()
    });

    // The user clicks the row of that workspace, which is no use of the notice's control.
    click_workspace('2', &mut cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('2')));
    assert_eq!(notice_text(&shown, &mut cx), None);
    assert_eq!(drawn("frame-turn-notice", &mut cx), None);

    // Leaving it again does not bring the old notice back.
    click_workspace('1', &mut cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));
    assert_eq!(notice_text(&shown, &mut cx), None);
    assert_eq!(drawn("frame-turn-notice", &mut cx), None);
}

#[gpui_kit::test]
fn the_key_of_the_notice_with_no_notice_changes_nothing_and_a_turn_of_a_session_not_listed_still_shows(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(cx);
    cx.simulate_keystrokes(&pressed::<ShowFinishedTurn>());
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));

    // The end of a turn arrives for a session the list does not hold yet: the notice
    // says what it can.
    shown.keeper.tell(turn_ended('4', 'x', 'a'));
    wait_until(&mut cx, "the notice", |cx| {
        notice_text(&shown, cx).is_some()
    });
    assert_eq!(
        notice_text(&shown, &mut cx).as_deref(),
        Some("An agent finished a turn in docs")
    );
    // The key did nothing before: no workspace was told to the Keeper by it.
    assert_eq!(shown.keeper.shown_workspace(), Some(id('1')));
}

#[gpui_kit::test]
fn there_are_two_ways_to_quit_the_plain_one_is_what_the_usual_key_runs_and_asks_the_keeper_to_stop_nothing(
    cx: &mut TestAppContext,
) {
    // The table the menu will be built from: two rows, one for each way.
    let table = controls();
    let quit = table
        .iter()
        .find(|control| (control.action)().name() == Quit.name())
        .expect("a row for the plain quit");
    let stop = table
        .iter()
        .find(|control| (control.action)().name() == QuitAndStopAgents.name())
        .expect("a row for the quit that stops the agents");
    assert_eq!(quit.title, "Quit");
    assert_eq!(quit.keystroke, "secondary-q", "the usual quit key");
    assert_eq!(quit.sends, None, "the plain quit sends nothing");
    assert_eq!(stop.title, "Quit and Stop Agents");
    assert_eq!(stop.keystroke, "secondary-alt-q");
    assert_eq!(stop.sends, Some(Sends::StopEverything));
    assert_ne!(quit.keystroke, stop.keystroke);

    let (shown, mut cx) = showing(cx);
    // The Window program handles `Quit` for the app; the test plays that part and counts.
    let quit_ran = Rc::new(Cell::new(0));
    cx.update(|_, cx| {
        let quit_ran = quit_ran.clone();
        cx.on_action(move |_: &Quit, _| quit_ran.set(quit_ran.get() + 1));
    });

    // The key the table gives is bound to the plain quit.
    // The parser takes a text that lives as long as the program.
    let usual = Keystroke::parse(Box::leak(pressed::<Quit>().into_boxed_str())).expect("a key");
    // Asked in the context of the Frame, where the keys of the table work on every system:
    // off macOS the key is bound in that context, and the window's own answer depends on a
    // stack of contexts the last frame left.
    let context = KeyContext::parse(KEY_CONTEXT).expect("the context of the Frame");
    let bound = cx.update(|window, _| window.bindings_for_action_in_context(&Quit, context));
    assert!(
        bound.iter().any(|binding| {
            let keys = binding.keystrokes();
            keys.len() == 1 && keys[0].key() == usual.key && keys[0].modifiers() == &usual.modifiers
        }),
        "the usual key is not bound to the plain quit: {bound:?}"
    );

    // Pressing it runs the plain quit once...
    cx.simulate_keystrokes(&pressed::<Quit>());
    assert_eq!(quit_ran.get(), 1);
    // ...and asks the Keeper to stop nothing. What the Window sends next is a request for
    // the workspace on screen; it was written after anything the quit could have sent and
    // is read in order, so once it is heard, a stop frame sent before it would be counted.
    click_workspace('2', &mut cx);
    wait_until(&mut cx, "the Keeper told of workspace 2", |_| {
        shown.keeper.shown_workspace() == Some(id('2'))
    });
    assert_eq!(shown.keeper.stops_heard(), 0);
    assert_eq!(quit_ran.get(), 1);

    // The other way stops everything and leaves by itself, not through the plain quit.
    cx.simulate_keystrokes(&pressed::<QuitAndStopAgents>());
    wait_until(&mut cx, "the stop frame", |_| {
        shown.keeper.stops_heard() == 1
    });
    assert_eq!(quit_ran.get(), 1, "the second way is no plain quit");
}

/// A window whose Keeper says this hello, with the note about it drawn when it is older.
fn meeting(keeper: Hello, cx: &mut TestAppContext) -> (Shown, VisualTestContext) {
    let (shown, cx) = open_frame_with_keeper(keeper, WIDE, Vec::new(), cx);
    cx.run_until_parked();
    (shown, cx)
}

fn hello(protocol: u16, app_version: &str) -> Hello {
    Hello {
        protocol,
        pid: 1,
        app_version: app_version.to_owned(),
    }
}

#[gpui_kit::test]
fn the_note_about_a_keeper_of_another_protocol_offers_the_quit_that_stops_everything(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = meeting(hello(PROTOCOL_VERSION - 1, "0.0.1"), cx);
    assert!(drawn("frame-keeper-note", &mut cx).is_some());
    let offer = drawn("frame-keeper-note-stop", &mut cx).expect("the offer is drawn");
    let note = drawn("frame-keeper-note", &mut cx).expect("the note");
    assert!(
        note.left() <= offer.left()
            && offer.right() <= note.right()
            && note.top() <= offer.top()
            && offer.bottom() <= note.bottom(),
        "the offer {offer:?} lies outside the note {note:?}"
    );
    // Nothing was sent before the click.
    cx.run_until_parked();
    assert_eq!(shown.keeper.stops_heard(), 0);

    cx.simulate_click(offer.center(), Modifiers::none());
    wait_until(&mut cx, "the stop frame", |_| {
        shown.keeper.stops_heard() == 1
    });
}

#[gpui_kit::test]
fn the_note_about_an_older_keeper_of_the_same_protocol_offers_it_too(cx: &mut TestAppContext) {
    let (shown, mut cx) = meeting(hello(PROTOCOL_VERSION, "0.0.1"), cx);
    let offer = drawn("frame-keeper-note-stop", &mut cx).expect("the offer is drawn");
    cx.simulate_click(offer.center(), Modifiers::none());
    wait_until(&mut cx, "the stop frame", |_| {
        shown.keeper.stops_heard() == 1
    });
}

#[gpui_kit::test]
fn a_keeper_of_the_same_version_has_no_note_and_so_no_offer(cx: &mut TestAppContext) {
    let (_shown, mut cx) = meeting(hello(PROTOCOL_VERSION, env!("CARGO_PKG_VERSION")), cx);
    assert_eq!(drawn("frame-keeper-note", &mut cx), None);
    assert_eq!(drawn("frame-keeper-note-stop", &mut cx), None);
}
