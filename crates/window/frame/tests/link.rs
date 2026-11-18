//! Component test: the Window's connection to the Keeper, through what every part of the
//! Window uses of it. The test plays the Keeper on the other end of a socket pair.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{AppContext, Entity, TestAppContext};
use yaar_frame::{AskError, KeeperLink, LinkStatus};
use yaar_link::{
    Fact, MAX_FRAME, Message, PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId, Request,
    RequestId, WorkspaceId,
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
