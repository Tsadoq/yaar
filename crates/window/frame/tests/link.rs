//! Component test: the Window's connection to the Keeper, through what every part of the
//! Window uses of it. The test plays the Keeper on the other end of a socket pair.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use gpui_kit::{AppContext, Entity, TestAppContext};
use yaar_frame::{AskError, KeeperLink, LinkStatus, TerminalOutput};
use yaar_link::{
    Ending, Fact, MAX_FRAME, Message, PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId,
    Request, RequestId, TerminalId, WorkspaceId,
};

use common::{KeeperEnd, answer, wait_until};

fn repositories() -> Vec<Repository> {
    vec![Repository {
        id: RepositoryId::new("0123456789abcdef0123456789abcdef"),
        name: "yaar".to_owned(),
        workspaces: Vec::new(),
    }]
}

fn workspace(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

fn get_changes(digit: char) -> Request {
    Request::GetChanges {
        workspace: workspace(digit),
    }
}

fn terminal(digit: char) -> TerminalId {
    TerminalId::new(digit.to_string().repeat(32))
}

fn status(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> LinkStatus {
    link.read_with(cx, |link, _| link.status().clone())
}

/// Every chunk of output a terminal holds, and whether its output has ended after them.
/// Takes no waiting: what has not arrived is not there.
fn drain(output: &TerminalOutput) -> (Vec<Vec<u8>>, bool) {
    let mut chunks = Vec::new();
    while let Some(bytes) = output.ready() {
        chunks.push(bytes);
    }
    // One look at the end, which a waker that wakes nobody cannot be told apart from
    // waiting: a pending look says the output goes on.
    let mut next = pin!(output.next());
    let ended = matches!(
        next.as_mut()
            .poll(&mut std::task::Context::from_waker(Waker::noop())),
        Poll::Ready(None)
    );
    (chunks, ended)
}

/// A link with a Keeper of the Window's own protocol at the other end.
fn linked(cx: &mut TestAppContext) -> (Entity<KeeperLink>, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let keeper = KeeperEnd::attach(&link, cx);
    (link, keeper)
}

#[gpui_kit::test]
fn a_request_goes_out_as_a_frame_and_its_answer_reaches_the_asker(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    let asked = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    let id = keeper.expect(Request::ListWorkspaces, cx);
    keeper.reply(id, Ok(Reply::Workspaces(repositories())));
    assert_eq!(answer(asked, cx), Ok(Reply::Workspaces(repositories())));

    // A refusal reaches the asker as the Keeper worded it.
    let folder = PathBuf::from("/tmp/no repository here");
    let asked = link.update(cx, |link, cx| {
        link.ask(
            Request::OpenFolder {
                path: folder.clone(),
            },
            cx,
        )
    });
    let id = keeper.expect(
        Request::OpenFolder {
            path: folder.clone(),
        },
        cx,
    );
    let refusal = Refusal::NotARepository { path: folder };
    keeper.reply(id, Err(refusal.clone()));
    assert_eq!(answer(asked, cx), Err(AskError::Refused(refusal)));
}

#[gpui_kit::test]
fn requests_arrive_in_the_order_asked_and_each_answer_finds_its_asker(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    // All three are asked before the app runs again: the order is that of the calls.
    let (first, second, third) = link.update(cx, |link, cx| {
        (
            link.ask(get_changes('1'), cx),
            link.ask(get_changes('2'), cx),
            link.ask(get_changes('3'), cx),
        )
    });
    let first_id = keeper.expect(get_changes('1'), cx);
    let second_id = keeper.expect(get_changes('2'), cx);
    let third_id = keeper.expect(get_changes('3'), cx);
    assert!(
        first_id != second_id && second_id != third_id && first_id != third_id,
        "requests that wait together have ids of their own"
    );

    // Answered out of order, each by its id.
    let refused = |digit| Refusal::UnknownWorkspace {
        workspace: workspace(digit),
    };
    keeper.reply(third_id, Err(refused('3')));
    keeper.reply(first_id, Err(refused('1')));
    keeper.reply(second_id, Ok(Reply::Done));
    assert_eq!(answer(first, cx), Err(AskError::Refused(refused('1'))));
    assert_eq!(answer(second, cx), Ok(Reply::Done));
    assert_eq!(answer(third, cx), Err(AskError::Refused(refused('3'))));
}

#[gpui_kit::test]
fn every_fact_reaches_a_subscriber_in_the_order_it_arrived(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let heard = heard.clone();
        cx.subscribe(&link, move |_, fact: &Fact, _| {
            heard.borrow_mut().push(fact.clone())
        })
    });

    let told = vec![
        Fact::ChangesChanged {
            workspace: workspace('1'),
        },
        Fact::Workspaces(repositories()),
        Fact::ChangesChanged {
            workspace: workspace('2'),
        },
    ];
    for fact in &told {
        keeper.tell(fact.clone());
    }
    wait_until(cx, "three facts", |_| heard.borrow().len() >= told.len());
    assert_eq!(*heard.borrow(), told);
}

#[gpui_kit::test]
fn a_closed_connection_answers_every_waiting_ask_and_every_later_one(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    let (first, second) = link.update(cx, |link, cx| {
        (
            link.ask(get_changes('1'), cx),
            link.ask(get_changes('2'), cx),
        )
    });
    // The Keeper has both before it goes.
    keeper.expect(get_changes('1'), cx);
    keeper.expect(get_changes('2'), cx);
    keeper.close();

    assert_eq!(answer(first, cx), Err(AskError::NotLinked));
    assert_eq!(answer(second, cx), Err(AskError::NotLinked));
    let later = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    assert_eq!(answer(later, cx), Err(AskError::NotLinked));
}

#[gpui_kit::test]
fn nothing_is_asked_while_no_keeper_of_the_same_version_is_reached(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let ask = |cx: &mut TestAppContext| {
        let asked = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
        answer(asked, cx)
    };

    // Not reached yet.
    assert_eq!(ask(cx), Err(AskError::NotLinked));

    // Reached, and of another version: it may be told to stop and nothing else.
    let other = KeeperEnd::attach_speaking(PROTOCOL_VERSION + 1, &link, cx);
    assert!(matches!(
        link.read_with(cx, |link, _| link.status().clone()),
        LinkStatus::OtherVersion(_)
    ));
    assert_eq!(ask(cx), Err(AskError::NotLinked));

    // Not reachable.
    link.update(cx, |link, cx| {
        link.attached(Err("no Keeper".to_owned()), cx)
    });
    assert_eq!(ask(cx), Err(AskError::NotLinked));

    // That outcome let the other Keeper go, and its end shows what it was sent: nothing.
    assert_eq!(other.rest(cx), Vec::<Message>::new());
}

#[gpui_kit::test]
fn a_keeper_reached_anew_takes_the_place_of_the_one_before(cx: &mut TestAppContext) {
    let (link, mut before) = linked(cx);
    let waiting = link.update(cx, |link, cx| link.ask(get_changes('1'), cx));
    before.expect(get_changes('1'), cx);

    let mut after = KeeperEnd::attach(&link, cx);

    // Who waited on the first is told, and the first is let go with nothing more sent.
    assert_eq!(answer(waiting, cx), Err(AskError::NotLinked));
    assert_eq!(before.rest(cx), Vec::<Message>::new());

    let asked = link.update(cx, |link, cx| link.ask(get_changes('2'), cx));
    let id = after.expect(get_changes('2'), cx);
    after.reply(id, Ok(Reply::Done));
    assert_eq!(answer(asked, cx), Ok(Reply::Done));
}

#[gpui_kit::test]
fn a_keeper_that_stops_speaking_the_link_is_let_go(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    let waiting = link.update(cx, |link, cx| link.ask(get_changes('1'), cx));
    keeper.expect(get_changes('1'), cx);

    // A frame of one byte, of a kind the Link does not have.
    keeper.send_raw(&[1, 0, 0, 0, 99]);

    assert_eq!(answer(waiting, cx), Err(AskError::NotLinked));
    assert_eq!(keeper.rest(cx), Vec::<Message>::new());
    assert_eq!(
        status(&link, cx),
        LinkStatus::Lost("unknown kind of frame: 99".to_owned())
    );
}

#[gpui_kit::test]
fn a_request_too_long_for_a_frame_is_refused_and_the_connection_stays(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    let too_long = link.update(cx, |link, cx| {
        link.ask(
            Request::SetBase {
                workspace: workspace('1'),
                base: "b".repeat(MAX_FRAME as usize),
            },
            cx,
        )
    });
    assert!(matches!(
        answer(too_long, cx),
        Err(AskError::Refused(Refusal::Failed { .. }))
    ));

    // Nothing of it was sent: the next request is the first the Keeper reads.
    let asked = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    let id: RequestId = keeper.expect(Request::ListWorkspaces, cx);
    keeper.reply(id, Ok(Reply::Done));
    assert_eq!(answer(asked, cx), Ok(Reply::Done));
}

/// Keeps the app running until the link is lost, and gives the reason.
fn lost(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> String {
    wait_until(cx, "the link to be lost", |cx| {
        matches!(status(link, cx), LinkStatus::Lost(_))
    });
    match status(link, cx) {
        LinkStatus::Lost(why) => why,
        other => panic!("{other:?} is not a lost link"),
    }
}

#[gpui_kit::test]
fn a_connection_that_ends_makes_the_status_lost_and_tells_the_observers(cx: &mut TestAppContext) {
    let (link, keeper) = linked(cx);
    let told = Rc::new(RefCell::new(0));
    let _observing = cx.update(|cx| {
        let told = told.clone();
        cx.observe(&link, move |_, _| *told.borrow_mut() += 1)
    });
    cx.run_until_parked();
    let before = *told.borrow();

    keeper.close();

    assert_eq!(lost(&link, cx), "the other side closed the connection");
    assert!(*told.borrow() > before, "the observers were not told");
}

#[gpui_kit::test]
fn a_keeper_that_says_what_a_keeper_does_not_say_is_lost_with_that_reason(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    keeper.send(&Message::Shutdown);

    assert!(lost(&link, cx).contains("does not say"));
}

#[gpui_kit::test]
fn facts_the_keeper_told_before_it_went_are_heard_before_the_loss(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    let order = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let order = order.clone();
        cx.subscribe(&link, move |_, _: &Fact, _| order.borrow_mut().push("fact"))
    });
    let _observing = cx.update(|cx| {
        let order = order.clone();
        let link = link.clone();
        cx.observe(&link.clone(), move |_, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Lost(_)) {
                order.borrow_mut().push("lost");
            }
        })
    });

    keeper.tell(Fact::ChangesChanged {
        workspace: workspace('1'),
    });
    keeper.close();

    lost(&link, cx);
    assert_eq!(*order.borrow(), vec!["fact", "lost"]);
}

#[gpui_kit::test]
fn a_keeper_reached_anew_is_not_lost_with_the_one_it_replaced(cx: &mut TestAppContext) {
    let (link, before) = linked(cx);
    let after = KeeperEnd::attach(&link, cx);

    // The first connection ends, and its end is heard by nobody.
    assert_eq!(before.rest(cx), Vec::<Message>::new());
    cx.run_until_parked();

    assert!(matches!(status(&link, cx), LinkStatus::Connected(_)));
    drop(after);
}

#[gpui_kit::test]
fn input_goes_out_in_frames_between_the_requests_in_the_order_given(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    let _first = link.update(cx, |link, cx| {
        let first = link.ask(get_changes('1'), cx);
        link.send_input(&terminal('1'), b"ls".to_vec());
        link.send_input(&terminal('2'), b"pwd\n".to_vec());
        first
    });
    let _second = link.update(cx, |link, cx| link.ask(get_changes('2'), cx));

    keeper.expect(get_changes('1'), cx);
    assert_eq!(keeper.input(cx), (terminal('1'), b"ls".to_vec()));
    assert_eq!(keeper.input(cx), (terminal('2'), b"pwd\n".to_vec()));
    keeper.expect(get_changes('2'), cx);
}

#[gpui_kit::test]
fn nothing_is_sent_for_no_bytes(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    link.update(cx, |link, _| link.send_input(&terminal('1'), Vec::new()));
    let _asked = link.update(cx, |link, cx| link.ask(get_changes('1'), cx));

    // The first thing the Keeper reads is the request.
    keeper.expect(get_changes('1'), cx);
}

#[gpui_kit::test]
fn a_paste_of_twenty_mebibytes_arrives_whole_in_order_in_frames_of_at_most_64_kib(
    cx: &mut TestAppContext,
) {
    const PIECE: usize = 64 * 1024;
    let (link, mut keeper) = linked(cx);
    // Every byte tells where it stood: a lost, doubled or swapped piece shows.
    let pasted: Vec<u8> = (0..20 * 1024 * 1024 + 7)
        .map(|n: usize| (n % 251) as u8 ^ (n / PIECE) as u8)
        .collect();

    link.update(cx, |link, _| {
        link.send_input(&terminal('1'), pasted.clone())
    });
    let _after = link.update(cx, |link, cx| link.ask(get_changes('1'), cx));

    let mut received = Vec::with_capacity(pasted.len());
    let mut frames = 0;
    while received.len() < pasted.len() {
        let (to, bytes) = keeper.input(cx);
        assert_eq!(to, terminal('1'));
        assert!(
            !bytes.is_empty() && bytes.len() <= PIECE,
            "a frame of {} bytes",
            bytes.len()
        );
        received.extend_from_slice(&bytes);
        frames += 1;
    }
    assert!(
        received == pasted,
        "the paste did not arrive as it was sent"
    );
    assert_eq!(frames, pasted.len().div_ceil(PIECE));
    // The request given after the paste leaves after all of it.
    keeper.expect(get_changes('1'), cx);
}

#[gpui_kit::test]
fn input_is_dropped_while_no_keeper_of_the_same_version_is_reached(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let send = |cx: &mut TestAppContext| {
        link.update(cx, |link, _| {
            link.send_input(&terminal('1'), b"ls".to_vec())
        })
    };

    // Not reached yet.
    send(cx);

    // Reached, and of another version.
    let other = KeeperEnd::attach_speaking(PROTOCOL_VERSION + 1, &link, cx);
    send(cx);

    link.update(cx, |link, cx| {
        link.attached(Err("no Keeper".to_owned()), cx)
    });
    send(cx);
    assert_eq!(other.rest(cx), Vec::<Message>::new());
}

#[gpui_kit::test]
fn input_is_dropped_once_the_connection_is_lost(cx: &mut TestAppContext) {
    let (link, keeper) = linked(cx);
    keeper.close();
    lost(&link, cx);

    // Nothing to send it on, and nothing breaks.
    link.update(cx, |link, _| {
        link.send_input(&terminal('1'), b"ls".to_vec())
    });
    let later = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    assert_eq!(answer(later, cx), Err(AskError::NotLinked));
}

#[gpui_kit::test]
fn a_keeper_that_reads_nothing_is_let_go_after_the_timeout_and_nothing_waits_for_it(
    cx: &mut TestAppContext,
) {
    let link = cx.new(|_| KeeperLink::new());
    let keeper_end = KeeperEnd::attach_deaf(&link, cx);

    // Far more than a socket holds: the writing thread blocks on it.
    let before = Instant::now();
    let waiting = link.update(cx, |link, cx| {
        link.send_input(&terminal('1'), vec![b'x'; 8 * 1024 * 1024]);
        link.ask(Request::ListWorkspaces, cx)
    });
    assert!(
        before.elapsed() < Duration::from_secs(3),
        "the calls waited for the Keeper"
    );
    cx.run_until_parked();
    assert!(
        matches!(status(&link, cx), LinkStatus::Connected(_)),
        "a Keeper is not let go before the timeout"
    );

    let why = lost(&link, cx);
    assert!(why.contains("took nothing"), "{why}");
    // The request that waited behind the input is told it will not be answered.
    assert_eq!(answer(waiting, cx), Err(AskError::NotLinked));
    drop(keeper_end);
}

#[gpui_kit::test]
fn output_that_arrives_before_it_is_taken_waits_and_is_given_in_order(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    // Two terminals, written to before anybody asks for either.
    keeper.output(&terminal('1'), b"$ ");
    keeper.output(&terminal('2'), b"other");
    keeper.output(&terminal('1'), b"ls\r\n");
    // A fact is a fence: the thread that reads has gone through all of the above.
    let heard = Rc::new(RefCell::new(0));
    let _subscription = cx.update(|cx| {
        let heard = heard.clone();
        cx.subscribe(&link, move |_, _: &Fact, _| *heard.borrow_mut() += 1)
    });
    keeper.tell(Fact::ChangesChanged {
        workspace: workspace('1'),
    });
    wait_until(cx, "the fence", |_| *heard.borrow() > 0);

    let first = link
        .update(cx, |link, _| link.take_output(&terminal('1')))
        .expect("output of the first terminal");
    assert_eq!(
        drain(&first),
        (vec![b"$ ".to_vec(), b"ls\r\n".to_vec()], false)
    );
    let second = link
        .update(cx, |link, _| link.take_output(&terminal('2')))
        .expect("output of the second terminal");
    assert_eq!(drain(&second), (vec![b"other".to_vec()], false));

    // What comes after it is taken reaches the one who took it, and nobody else.
    keeper.output(&terminal('1'), b"more");
    let bytes = answer(
        cx.foreground_executor()
            .spawn(async move { first.next().await }),
        cx,
    );
    assert_eq!(bytes, Some(b"more".to_vec()));
    assert_eq!(drain(&second), (Vec::new(), false));
}

#[gpui_kit::test]
fn output_taken_before_any_arrives_is_given_when_it_does(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);

    let output = link
        .update(cx, |link, _| link.take_output(&terminal('1')))
        .expect("output of a terminal nothing was heard from");
    keeper.output(&terminal('1'), b"hello");

    let bytes = answer(
        cx.foreground_executor()
            .spawn(async move { output.next().await }),
        cx,
    );
    assert_eq!(bytes, Some(b"hello".to_vec()));
}

#[gpui_kit::test]
fn the_output_of_a_terminal_is_given_once(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    keeper.output(&terminal('1'), b"$ ");

    let first = link.update(cx, |link, _| link.take_output(&terminal('1')));
    let second = link.update(cx, |link, _| link.take_output(&terminal('1')));

    assert!(first.is_some());
    assert!(second.is_none());
}

#[gpui_kit::test]
fn output_ends_after_its_last_bytes_before_the_end_of_the_terminal_is_heard(
    cx: &mut TestAppContext,
) {
    let (link, mut keeper) = linked(cx);
    let taken = link
        .update(cx, |link, _| link.take_output(&terminal('1')))
        .expect("output");
    let untaken = terminal('2');
    // What the one heard when the end of its terminal was told: all of it, and then the end.
    let at_the_end = Rc::new(RefCell::new(None));
    let _subscription = cx.update(|cx| {
        let at_the_end = at_the_end.clone();
        cx.subscribe(&link, move |_, fact: &Fact, _| {
            if matches!(fact, Fact::TerminalEnded { .. }) {
                *at_the_end.borrow_mut() = Some(drain(&taken));
            }
        })
    });

    keeper.output(&terminal('1'), b"bye\r\n");
    keeper.output(&untaken, b"still here");
    keeper.tell(Fact::TerminalEnded {
        terminal: terminal('1'),
        ending: Ending::Exited(0),
    });
    wait_until(cx, "the end of the terminal", |_| {
        at_the_end.borrow().is_some()
    });
    assert_eq!(
        at_the_end.take(),
        Some((vec![b"bye\r\n".to_vec()], true)),
        "the output is complete and over when the end is told"
    );

    // Bytes of an ended terminal are dropped, and the other terminal goes on.
    keeper.output(&terminal('1'), b"ghost");
    keeper.output(&untaken, b"!");
    let fence = link
        .update(cx, |link, _| link.take_output(&untaken))
        .expect("output of the other terminal");
    wait_until(cx, "the other terminal's output", |_| {
        fence.ready().is_some_and(|first| first == b"still here")
    });
    wait_until(cx, "the bytes after it", |_| {
        fence.ready().is_some_and(|next| next == b"!")
    });
    let ended = link
        .update(cx, |link, _| link.take_output(&terminal('1')))
        .map(|output| drain(&output));
    assert_eq!(ended, None, "taken once already");
}

#[gpui_kit::test]
fn the_output_of_a_terminal_that_ended_untaken_is_still_there_to_take(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    keeper.output(&terminal('1'), b"bye");
    keeper.output(&terminal('1'), b" now");
    keeper.tell(Fact::TerminalEnded {
        terminal: terminal('1'),
        ending: Ending::Exited(0),
    });
    keeper.output(&terminal('1'), b"ghost");
    let heard = Rc::new(RefCell::new(0));
    let _subscription = cx.update(|cx| {
        let heard = heard.clone();
        cx.subscribe(&link, move |_, _: &Fact, _| *heard.borrow_mut() += 1)
    });
    // The fence comes after the ghost.
    keeper.tell(Fact::ChangesChanged {
        workspace: workspace('1'),
    });
    wait_until(cx, "the fence", |_| *heard.borrow() >= 1);

    let output = link
        .update(cx, |link, _| link.take_output(&terminal('1')))
        .expect("output");
    assert_eq!(
        drain(&output),
        (vec![b"bye".to_vec(), b" now".to_vec()], true)
    );
}

#[gpui_kit::test]
fn when_the_connection_ends_every_output_ends_after_what_it_holds(cx: &mut TestAppContext) {
    let (link, mut keeper) = linked(cx);
    let taken = link
        .update(cx, |link, _| link.take_output(&terminal('1')))
        .expect("output");
    keeper.output(&terminal('1'), b"one");
    keeper.output(&terminal('2'), b"two");

    keeper.close();
    lost(&link, cx);

    assert_eq!(drain(&taken), (vec![b"one".to_vec()], true));
    // What was not taken yet is there to take, and ends too; so does one never heard of.
    let untaken = link
        .update(cx, |link, _| link.take_output(&terminal('2')))
        .expect("output");
    assert_eq!(drain(&untaken), (vec![b"two".to_vec()], true));
    let unknown = link
        .update(cx, |link, _| link.take_output(&terminal('3')))
        .expect("output");
    assert_eq!(drain(&unknown), (Vec::new(), true));
}
