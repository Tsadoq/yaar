//! Component test: the two shared models of slice 4, the agents and the terminals, the
//! workspace on screen told to the Keeper, and the stop frame, through what the panes use
//! of them. The test plays the Keeper on the other end of a socket pair: it answers what
//! the models ask, tells them facts, and reads back through the models.
//!
//! A negative clause is read after a barrier, see [`settle`].

mod common;

use std::time::{Duration, Instant};

use gpui_kit::{AppContext, Modifiers, TestAppContext, VisualTestContext, px, size};
use yaar_frame::{KeeperLink, LinkStatus, Models};
use yaar_link::{
    Ending, Fact, Hello, Message, PROTOCOL_VERSION, Program, Refusal, Repository, Request, Session,
    SessionId, SessionState, TerminalId, TerminalInfo, WorkspaceAgents, WorkspaceId, read_message,
};

use common::{KeeperEnd, drawn, id, open_frame, repository, wait_until, workspace};

/// A session of Claude Code in workspace `workspace`, in `terminal` when there is one.
fn session(name: char, workspace: char, terminal: Option<char>, state: SessionState) -> Session {
    Session {
        id: SessionId::new(name.to_string().repeat(32)),
        agent: "claude".to_owned(),
        title: format!("Claude Code {name}"),
        workspace: id(workspace),
        terminal: terminal.map(terminal_id),
        state,
        turn: None,
        review: None,
        attached: None,
        resumable: false,
        turn_from_user: false,
    }
}

/// The sessions of one workspace, which shows `shown`.
fn agents_of(
    workspace: char,
    shown: Option<SessionState>,
    sessions: Vec<Session>,
) -> WorkspaceAgents {
    WorkspaceAgents {
        workspace: id(workspace),
        shown,
        sessions,
    }
}

fn terminal_id(character: char) -> TerminalId {
    TerminalId::new(character.to_string().repeat(32))
}

/// A terminal of workspace `workspace` that runs the user's shell.
fn shell(terminal: char, workspace: char) -> TerminalInfo {
    TerminalInfo {
        id: terminal_id(terminal),
        workspace: id(workspace),
        program: Program::Shell,
    }
}

/// Three worktrees of one repository, which the Keeper lists; the first is on screen.
fn repositories() -> Vec<Repository> {
    vec![repository(
        'a',
        vec![
            workspace('1', "main"),
            workspace('2', "feature"),
            workspace('3', "fix"),
        ],
    )]
}

fn hello(protocol: u16) -> Hello {
    Hello {
        protocol,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    }
}

/// The models of a window whose Keeper the test plays. `stage` sets what the Keeper knows
/// before the Window reaches it, as the first answers hold it. The first question for the
/// list is answered with [`repositories`].
fn connected(stage: impl FnOnce(&KeeperEnd), cx: &mut TestAppContext) -> (Models, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_staged(hello(PROTOCOL_VERSION), &link, cx, stage);
    keeper.list(&repositories(), cx);
    wait_until(cx, "a workspace on screen", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.selected().is_some())
    });
    (models, keeper)
}

/// A barrier: returns when the Keeper has heard everything the Window sent before it, and
/// the Window has taken everything the Keeper sent before the answer to it. The Window
/// asks for the list again and the Keeper answers with a list that carries `mark`, which
/// the Window then holds. A count read after it is final for what came before, and what
/// the Keeper answered before it is taken in.
fn settle(mark: char, models: &Models, keeper: &mut KeeperEnd, cx: &mut TestAppContext) {
    models.workspaces.update(cx, |list, cx| list.refresh(cx));
    // The models of a Frame ask for more than the list: leave that unanswered.
    let asked = loop {
        let (asked, request) = keeper.request(cx);
        if request == Request::ListWorkspaces {
            break asked;
        }
    };
    let marked = vec![repository(mark, repositories()[0].workspaces.clone())];
    keeper.reply(asked, Ok(yaar_link::Reply::Workspaces(marked.clone())));
    wait_until(cx, "the list the barrier carries", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.repositories() == marked)
    });
}

fn shown_of(models: &Models, workspace: char, cx: &mut TestAppContext) -> Option<SessionState> {
    models
        .agents
        .read_with(cx, |list, _| list.shown(&id(workspace)))
}

#[gpui_kit::test]
fn the_agents_the_keeper_lists_when_reached_are_held_in_its_order_and_asked_for_once(
    cx: &mut TestAppContext,
) {
    let working = session('w', '1', Some('a'), SessionState::Works);
    let ended = session('e', '2', None, SessionState::NotRunning);
    let first = agents_of('1', Some(SessionState::Works), vec![working.clone()]);
    let second = agents_of('2', Some(SessionState::NotRunning), vec![ended.clone()]);

    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    models.agents.read_with(cx, |list, _| {
        assert!(list.all().is_empty(), "nothing is known before the Keeper");
    });
    let mut keeper = KeeperEnd::attach_staged(hello(PROTOCOL_VERSION), &link, cx, |keeper| {
        keeper.set_agents(vec![first.clone(), second.clone()]);
    });
    wait_until(cx, "the agents", |cx| {
        models
            .agents
            .read_with(cx, |list, _| list.all() == [first.clone(), second.clone()])
    });

    models.agents.read_with(cx, |list, _| {
        assert_eq!(list.of(&id('2')), Some(&second));
        assert_eq!(list.of(&id('3')), None, "no session ever reported in 3");
        assert_eq!(list.shown(&id('1')), Some(SessionState::Works));
        assert_eq!(list.shown(&id('3')), None);
        assert_eq!(list.session(&ended.id), Some(&ended));
        assert_eq!(list.session(&SessionId::new("z".repeat(32))), None);
        assert_eq!(list.in_terminal(&terminal_id('a')), Some(&working));
        assert_eq!(list.in_terminal(&terminal_id('b')), None);
    });

    // Asked once, when the Window reached the Keeper, and not again for the list.
    keeper.list(&repositories(), cx);
    settle('x', &models, &mut keeper, cx);
    assert_eq!(keeper.agents_asked(), 1);
}

#[gpui_kit::test]
fn a_fact_replaces_the_entry_of_its_workspace_or_adds_one_and_asks_nothing(
    cx: &mut TestAppContext,
) {
    let working = session('w', '1', None, SessionState::Works);
    let (models, mut keeper) = connected(
        |keeper| {
            keeper.set_agents(vec![agents_of(
                '1',
                Some(SessionState::Works),
                vec![working.clone()],
            )]);
        },
        cx,
    );
    wait_until(cx, "the first agents", |cx| {
        shown_of(&models, '1', cx) == Some(SessionState::Works)
    });

    let done = Session {
        state: SessionState::Unreviewed,
        ..working
    };
    keeper.tell(Fact::AgentsChanged(agents_of(
        '1',
        Some(SessionState::Unreviewed),
        vec![done.clone()],
    )));
    wait_until(cx, "the new state of workspace 1", |cx| {
        shown_of(&models, '1', cx) == Some(SessionState::Unreviewed)
    });
    let other = agents_of(
        '3',
        Some(SessionState::WaitsForPermission),
        vec![session('p', '3', None, SessionState::WaitsForPermission)],
    );
    keeper.tell(Fact::AgentsChanged(other.clone()));
    wait_until(cx, "workspace 3 among the agents", |cx| {
        shown_of(&models, '3', cx) == Some(SessionState::WaitsForPermission)
    });

    models.agents.read_with(cx, |list, _| {
        assert_eq!(list.all().len(), 2, "one entry for each workspace");
        assert_eq!(list.all()[0].sessions, vec![done.clone()]);
        assert_eq!(list.all()[1], other, "a new workspace comes last");
    });
    // Nothing was asked for the facts: the facts carry what they say.
    settle('x', &models, &mut keeper, cx);
    assert_eq!(keeper.agents_asked(), 1);
}

#[gpui_kit::test]
fn of_two_answers_for_the_agents_only_the_one_to_the_last_question_counts_and_a_fact_on_the_way_asks_again(
    cx: &mut TestAppContext,
) {
    let before = session('w', '1', None, SessionState::Works);
    let (models, mut keeper) = connected(
        |keeper| {
            keeper.set_agents(vec![agents_of(
                '1',
                Some(SessionState::Works),
                vec![before.clone()],
            )]);
            keeper.hold_agents();
        },
        cx,
    );
    wait_until(cx, "the first question to be heard", |_| {
        keeper.held_agents() == 1
    });

    // The turn ended while the answer was on its way: the Keeper knows it now, tells it,
    // and answers the next question with it.
    let after = Session {
        state: SessionState::Unreviewed,
        ..before
    };
    let now = agents_of('1', Some(SessionState::Unreviewed), vec![after]);
    keeper.set_agents(vec![now.clone()]);
    keeper.tell(Fact::AgentsChanged(now.clone()));
    wait_until(cx, "the fact", |cx| {
        shown_of(&models, '1', cx) == Some(SessionState::Unreviewed)
    });
    wait_until(cx, "the question to be asked again", |_| {
        keeper.held_agents() == 2
    });

    // The old answer, made before the turn ended, comes late. It is not taken.
    keeper.release_agents(1);
    settle('x', &models, &mut keeper, cx);
    assert_eq!(shown_of(&models, '1', cx), Some(SessionState::Unreviewed));

    keeper.release_agents(1);
    settle('y', &models, &mut keeper, cx);
    models
        .agents
        .read_with(cx, |list, _| assert_eq!(list.all(), [now]));
    assert_eq!(keeper.agents_asked(), 2, "asked again once, for one fact");
}

#[gpui_kit::test]
fn the_terminals_the_keeper_lists_when_reached_join_the_ones_this_window_opens_once_each_and_leave_when_they_end(
    cx: &mut TestAppContext,
) {
    let (one, two) = (shell('1', '1'), shell('2', '2'));
    let (models, mut keeper) = connected(
        |keeper| keeper.set_terminals(vec![one.clone(), two.clone()]),
        cx,
    );
    wait_until(cx, "the terminals", |cx| {
        models
            .terminals
            .read_with(cx, |list, _| list.terminals() == [one.clone(), two.clone()])
    });
    assert_eq!(keeper.terminals_asked(), 1);

    // This Window opens one, and says so twice.
    let three = shell('3', '1');
    for _ in 0..2 {
        models
            .terminals
            .update(cx, |list, cx| list.opened(three.clone(), cx));
    }
    models.terminals.read_with(cx, |list, _| {
        assert_eq!(list.terminals(), [one.clone(), two.clone(), three.clone()]);
        assert_eq!(list.of(&id('1')), vec![&one, &three]);
        assert_eq!(list.of(&id('2')), vec![&two]);
        assert!(list.of(&id('3')).is_empty());
    });

    // One ends. A terminal nobody listed that ends changes nothing.
    keeper.tell(Fact::TerminalEnded {
        terminal: terminal_id('9'),
        ending: Ending::Exited(0),
    });
    keeper.tell(Fact::TerminalEnded {
        terminal: terminal_id('1'),
        ending: Ending::Exited(0),
    });
    wait_until(cx, "the terminal to leave", |cx| {
        models.terminals.read_with(cx, |list, _| {
            list.terminals() == [two.clone(), three.clone()]
        })
    });
    // Nothing was asked for the opening or the end: the Window knows them.
    settle('x', &models, &mut keeper, cx);
    assert_eq!(keeper.terminals_asked(), 1);
}

#[gpui_kit::test]
fn of_two_answers_for_the_terminals_only_the_one_to_the_last_question_counts_whether_a_terminal_ended_or_was_opened_on_the_way(
    cx: &mut TestAppContext,
) {
    let (one, two) = (shell('1', '1'), shell('2', '1'));
    let (models, mut keeper) = connected(
        |keeper| {
            keeper.set_terminals(vec![one.clone()]);
            keeper.hold_terminals();
        },
        cx,
    );
    wait_until(cx, "the first question to be heard", |_| {
        keeper.held_terminals() == 1
    });

    // While that answer is on its way, `one` ends and this Window opens `two`. The
    // Keeper answers each question with what it knows when it hears it: none after the
    // end, then `two` after the opening.
    keeper.set_terminals(Vec::new());
    keeper.tell(Fact::TerminalEnded {
        terminal: terminal_id('1'),
        ending: Ending::Exited(0),
    });
    wait_until(cx, "the end", |cx| {
        models
            .terminals
            .read_with(cx, |list, _| list.terminals().is_empty())
    });
    wait_until(cx, "the question to be asked again", |_| {
        keeper.held_terminals() == 2
    });
    keeper.set_terminals(vec![two.clone()]);
    models
        .terminals
        .update(cx, |list, cx| list.opened(two.clone(), cx));
    wait_until(cx, "the question to be asked once more", |_| {
        keeper.held_terminals() == 3
    });

    // The first answer still lists `one` and the second lists none, each late and each
    // read alone after a barrier: neither is taken. Only the last, `two`, is.
    for (mark, answer) in [('x', "first"), ('y', "second"), ('z', "last")] {
        keeper.release_terminals(1);
        settle(mark, &models, &mut keeper, cx);
        models.terminals.read_with(cx, |list, _| {
            assert_eq!(
                list.terminals(),
                std::slice::from_ref(&two),
                "after the {answer} answer"
            )
        });
    }
}

/// Clicks the row of a workspace in the sidebar where it was drawn.
fn click_workspace(workspace: char, cx: &mut VisualTestContext) {
    let row = drawn(&format!("sidebar-workspace-{}", id(workspace)), cx)
        .expect("the row of the workspace is drawn");
    cx.simulate_click(row.center(), Modifiers::none());
}

#[gpui_kit::test]
fn a_click_that_selects_a_workspace_tells_the_keeper_which_is_on_screen_once_for_each_change(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = open_frame(size(px(1280.), px(800.)), Vec::new(), cx);
    shown.keeper.list(&repositories(), &mut cx);
    wait_until(&mut cx, "the sidebar to show the list", |cx| {
        // One row for the repository and one for each of its three worktrees.
        shown
            .sidebar
            .read_with(cx, |sidebar, cx| sidebar.rows(cx))
            .len()
            == 4
    });
    // When the link connected nothing was on screen; then the first of the list was.
    wait_until(&mut cx, "the first workspace told", |_| {
        shown.keeper.shown_told() == [None, Some(id('1'))]
    });

    click_workspace('2', &mut cx);
    // The same workspace again changes nothing, so nothing is told for it.
    click_workspace('2', &mut cx);
    click_workspace('3', &mut cx);
    wait_until(&mut cx, "the third workspace told", |_| {
        shown.keeper.shown_workspace() == Some(id('3'))
    });
    assert_eq!(
        shown.keeper.shown_told(),
        [None, Some(id('1')), Some(id('2')), Some(id('3'))]
    );
}

/// What each `ShowWorkspace` of a Keeper named, once the Window has said what it will.
fn told_after_settling(
    mark: char,
    models: &Models,
    keeper: &mut KeeperEnd,
    cx: &mut TestAppContext,
) -> Vec<Option<WorkspaceId>> {
    // Twice: the list the first carries may itself make the Window tell something, and
    // the second barrier is after that.
    settle(mark, models, keeper, cx);
    settle(mark.to_ascii_uppercase(), models, keeper, cx);
    keeper.shown_told()
}

#[gpui_kit::test]
fn the_workspace_on_screen_is_told_again_when_the_list_moves_it_and_to_each_keeper_that_is_reached(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = connected(|_| {}, cx);
    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('3'), cx));
    // The list loses the workspace on screen: the first takes its place.
    keeper.tell(Fact::Workspaces(vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )]));
    wait_until(cx, "the first workspace back on screen", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&id('1')))
    });
    assert_eq!(
        told_after_settling('x', &models, &mut keeper, cx),
        [None, Some(id('1')), Some(id('3')), Some(id('1'))]
    );

    // The link is made to reach a Keeper again, as the Window never does by itself yet:
    // that Keeper knows nothing of what is on screen.
    let link = models.link.clone();
    let mut again = KeeperEnd::attach(&link, cx);
    again.list(&repositories(), cx);
    assert_eq!(
        told_after_settling('y', &models, &mut again, cx),
        [Some(id('1'))]
    );
}

#[gpui_kit::test]
fn a_keeper_that_refuses_the_workspace_on_screen_is_told_it_once_for_each_change_and_the_user_sees_no_refusal(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = connected(|keeper| keeper.refuse_show(Some(not_wired())), cx);
    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));
    // A refusal is no reason to tell it again, and none for the user: nothing was asked
    // of the app but to show a workspace.
    assert_eq!(
        told_after_settling('x', &models, &mut keeper, cx),
        [None, Some(id('1')), Some(id('2'))]
    );
    models
        .workspaces
        .read_with(cx, |list, _| assert_eq!(list.refusal(), None));
}

fn not_wired() -> Refusal {
    Refusal::Failed {
        reason: "not wired".to_owned(),
    }
}

#[gpui_kit::test]
fn a_keeper_of_another_protocol_is_told_nothing_of_the_workspace_on_screen_but_the_stop_frame_reaches_it(
    cx: &mut TestAppContext,
) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_speaking(PROTOCOL_VERSION + 1, &link, cx);
    link.read_with(cx, |link, _| {
        assert!(matches!(link.status(), LinkStatus::OtherVersion(_)));
    });
    models.workspaces.update(cx, |list, cx| list.refresh(cx));

    // The stop frame is the one thing every Keeper reads, and what it says first here:
    // nothing was sent before it, and so nothing was asked.
    link.update(cx, |link, _| link.stop_everything());
    keeper.shutdown(cx);
    assert!(keeper.shown_told().is_empty());
    assert_eq!(keeper.agents_asked(), 0);
    assert_eq!(keeper.terminals_asked(), 0);
}

#[gpui_kit::test]
fn the_stop_frame_reaches_the_keeper_after_everything_given_before_it(cx: &mut TestAppContext) {
    // So much input that the writing thread is certainly still busy with it when the stop
    // is given: a stop that did not wait its turn would overtake the input, or cut into a
    // frame being written.
    const SIZE: usize = 8 * 1024 * 1024;
    let link = cx.new(|_| KeeperLink::new());
    let mut keeper = KeeperEnd::attach(&link, cx);
    let terminal = terminal_id('t');
    let typed: Vec<u8> = (0..SIZE).map(|at| (at % 251) as u8).collect();

    let asked = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    link.update(cx, |link, _| link.send_input(&terminal, typed.clone()));
    link.update(cx, |link, _| link.stop_everything());

    assert_eq!(keeper.request(cx).1, Request::ListWorkspaces);
    wait_until(cx, "the stop frame", |_| keeper.stops_heard() == 1);
    // An input frame is counted before the test can read it, and so is the stop frame:
    // everything given before the stop was read before it.
    let read: Vec<u8> = keeper.inputs(&terminal).concat();
    assert_eq!(
        read.len(),
        SIZE,
        "the stop frame came before the input ended"
    );
    assert!(read == typed, "the input arrived changed or out of order");
    drop(asked);
}

#[gpui_kit::test]
fn a_stop_without_a_keeper_to_reach_is_a_stop_of_nothing(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    link.update(cx, |link, _| link.stop_everything());
    link.read_with(cx, |link, _| {
        assert_eq!(link.status(), &LinkStatus::Starting)
    });

    // A Keeper that has gone: the link is lost, and the frame goes nowhere.
    let keeper = KeeperEnd::attach(&link, cx);
    keeper.close();
    wait_until(cx, "the link to be lost", |cx| {
        link.read_with(cx, |link, _| matches!(link.status(), LinkStatus::Lost(_)))
    });
    link.update(cx, |link, _| link.stop_everything());
    link.read_with(cx, |link, _| {
        assert!(matches!(link.status(), LinkStatus::Lost(_)))
    });
}

#[gpui_kit::test]
fn the_stop_is_written_before_the_quit_goes_on_and_a_keeper_that_takes_nothing_holds_it_up_only_for_the_time_given(
    cx: &mut TestAppContext,
) {
    // A Keeper that reads: the call returns once the frame has left, so the frame is
    // whole at the Keeper's end by then, with no waiting for it.
    let link = cx.new(|_| KeeperLink::new());
    let mut reading = KeeperEnd::attach_deaf(&link, cx);
    let left = link.update(cx, |link, _| {
        link.stop_everything_and_wait(Duration::from_secs(5))
    });
    assert!(left, "the frame left");
    reading.set_nonblocking(true).expect("a nonblocking end");
    match read_message(&mut reading) {
        Ok(Message::Shutdown) => {}
        Ok(other) => panic!("the Keeper read {other:?}, and the stop frame was expected"),
        Err(error) => panic!("the call returned before the frame was at the Keeper: {error}"),
    }

    // With no connection there is nothing to wait for.
    let alone = cx.new(|_| KeeperLink::new());
    let started = Instant::now();
    assert!(!alone.update(cx, |link, _| {
        link.stop_everything_and_wait(Duration::from_secs(5))
    }));
    assert!(started.elapsed() < Duration::from_secs(2));

    // A Keeper that went before the frame was written: the writing thread reaches the
    // frame and cannot write it. The call says so, instead of saying it left. This is
    // the case that guards the order of the write and the signal: with the signal first,
    // the answer above is still true, as the writing thread wins that race. The link
    // has not yet taken in that the Keeper is gone: nothing ran the app since.
    let link = cx.new(|_| KeeperLink::new());
    let gone = KeeperEnd::attach_deaf(&link, cx);
    drop(gone);
    assert!(
        !link.update(cx, |link, _| {
            link.stop_everything_and_wait(Duration::from_secs(5))
        }),
        "the frame cannot have left for a Keeper that is gone"
    );

    // A Keeper that takes nothing: its buffer fills, the writing thread waits for room, and
    // the call gives up after the time it was given instead of holding the quit up.
    let link = cx.new(|_| KeeperLink::new());
    let deaf = KeeperEnd::attach_deaf(&link, cx);
    let terminal = terminal_id('t');
    link.update(cx, |link, _| {
        link.send_input(&terminal, vec![0; 16 * 1024 * 1024])
    });
    let started = Instant::now();
    let left = link.update(cx, |link, _| {
        link.stop_everything_and_wait(Duration::from_millis(300))
    });
    let waited = started.elapsed();
    assert!(
        !left,
        "the frame cannot have left a Keeper that reads nothing"
    );
    assert!(
        waited >= Duration::from_millis(300) && waited < Duration::from_secs(3),
        "waited {waited:?}"
    );
    // Dropping the Keeper's end lets the writing thread go.
    drop(deaf);
}
