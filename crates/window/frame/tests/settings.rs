mod common;

use std::io::{Cursor, ErrorKind, Read};
use std::net::Shutdown;
use std::time::{Duration, Instant};

use gpui_kit::{AppContext, Entity, TestAppContext};
use yaar_frame::{KeeperLink, Models, SettingList};
use yaar_link::{
    Fact, FlowForm, Hello, Message, PROTOCOL_VERSION, Refusal, Reply, Request, RequestId, Setting,
    read_message, write_message,
};

use common::{KeeperEnd, answer, id, repository, wait_until, workspace};

fn hello() -> Hello {
    Hello {
        protocol: PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    }
}

fn form(workspace: char, form: FlowForm) -> Setting {
    Setting::FlowForm {
        workspace: id(workspace),
        form,
    }
}

fn connected(stage: impl FnOnce(&KeeperEnd), cx: &mut TestAppContext) -> (Models, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_staged(hello(), &link, cx, stage);
    keeper.list(&[repository('a', vec![workspace('1', "main")])], cx);
    (models, keeper)
}

fn held(models: &Models, cx: &mut TestAppContext) -> Vec<Setting> {
    models.settings.read_with(cx, |list, _| list.all().to_vec())
}

/// A barrier: returns when the Keeper has heard everything the Window sent before it and the
/// Window has taken everything the Keeper sent before the answer. A count read after it is final
/// for what came before.
fn settle(mark: char, models: &Models, keeper: &mut KeeperEnd, cx: &mut TestAppContext) {
    models.workspaces.update(cx, |list, cx| list.refresh(cx));
    // The models of a Frame ask for more than the list: leave that unanswered.
    let asked = loop {
        let (asked, request) = keeper.request(cx);
        if request == Request::ListWorkspaces {
            break asked;
        }
    };
    let marked = vec![repository(mark, vec![workspace('1', "main")])];
    keeper.reply(asked, Ok(Reply::Workspaces(marked.clone())));
    wait_until(cx, "the list the barrier carries", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.repositories() == marked)
    });
}

#[gpui_kit::test]
fn the_settings_the_keeper_lists_when_reached_are_held_in_its_order_and_asked_for_once(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = connected(
        |keeper| {
            keeper.set_settings(vec![
                form('2', FlowForm::LiveDiff),
                form('1', FlowForm::Timeline),
            ]);
        },
        cx,
    );
    wait_until(cx, "the settings", |cx| !held(&models, cx).is_empty());

    assert_eq!(
        held(&models, cx),
        [form('2', FlowForm::LiveDiff), form('1', FlowForm::Timeline)]
    );
    models.settings.read_with(cx, |list, _| {
        assert_eq!(list.flow_form(&id('2')), FlowForm::LiveDiff);
        assert_eq!(list.flow_form(&id('1')), FlowForm::Timeline);
        assert_eq!(list.flow_form(&id('3')), FlowForm::Timeline, "none chosen");
    });
    settle('x', &models, &mut keeper, cx);
    assert_eq!(keeper.settings_asked(), 1);
}

#[gpui_kit::test]
fn a_fact_replaces_the_setting_of_its_workspace_or_adds_one_and_asks_nothing(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = connected(
        |keeper| keeper.set_settings(vec![form('1', FlowForm::Timeline)]),
        cx,
    );
    wait_until(cx, "the first settings", |cx| !held(&models, cx).is_empty());

    keeper.tell(Fact::SettingChanged(form('1', FlowForm::LiveDiff)));
    wait_until(cx, "the new form of workspace 1", |cx| {
        held(&models, cx) == [form('1', FlowForm::LiveDiff)]
    });
    keeper.tell(Fact::SettingChanged(form('3', FlowForm::LiveDiff)));
    wait_until(cx, "workspace 3 among the settings", |cx| {
        held(&models, cx).len() == 2
    });

    assert_eq!(
        held(&models, cx),
        [form('1', FlowForm::LiveDiff), form('3', FlowForm::LiveDiff)],
        "one setting for each workspace, a new one last"
    );
    keeper.tell(Fact::SettingChanged(form('3', FlowForm::LiveDiff)));
    settle('x', &models, &mut keeper, cx);
    assert_eq!(held(&models, cx).len(), 2);
    assert_eq!(keeper.settings_asked(), 1);
}

#[gpui_kit::test]
fn a_setting_set_shows_at_once_and_reaches_the_keeper_in_the_place_of_the_one_it_replaces(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = connected(
        |keeper| keeper.set_settings(vec![form('1', FlowForm::Timeline)]),
        cx,
    );
    wait_until(cx, "the first settings", |cx| !held(&models, cx).is_empty());

    models.settings.update(cx, |list, cx| {
        list.set(form('1', FlowForm::LiveDiff), cx);
        list.set(form('2', FlowForm::LiveDiff), cx);
    });
    assert_eq!(
        held(&models, cx),
        [form('1', FlowForm::LiveDiff), form('2', FlowForm::LiveDiff)]
    );

    settle('x', &models, &mut keeper, cx);
    assert_eq!(
        keeper.settings_set(),
        [form('1', FlowForm::LiveDiff), form('2', FlowForm::LiveDiff)]
    );
    assert_eq!(keeper.settings_asked(), 1);
    assert_eq!(
        held(&models, cx),
        [form('1', FlowForm::LiveDiff), form('2', FlowForm::LiveDiff)]
    );
}

#[gpui_kit::test]
fn a_setting_the_keeper_refuses_shows_at_once_and_goes_when_the_list_is_asked_again(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = connected(
        |keeper| keeper.set_settings(vec![form('2', FlowForm::LiveDiff)]),
        cx,
    );
    wait_until(cx, "the first settings", |cx| !held(&models, cx).is_empty());
    keeper.refuse_settings(Some(Refusal::Failed {
        reason: "not now".to_owned(),
    }));

    models
        .settings
        .update(cx, |list, cx| list.set(form('1', FlowForm::LiveDiff), cx));
    assert_eq!(
        held(&models, cx),
        [form('2', FlowForm::LiveDiff), form('1', FlowForm::LiveDiff)],
        "shown before the Keeper answered"
    );

    wait_until(cx, "the list to be asked again", |_| {
        keeper.settings_asked() == 2
    });
    wait_until(cx, "what the Keeper keeps", |cx| {
        held(&models, cx) == [form('2', FlowForm::LiveDiff)]
    });
    settle('x', &models, &mut keeper, cx);
    assert_eq!(keeper.settings_asked(), 2, "asked again once");
    assert_eq!(keeper.settings_set(), [form('1', FlowForm::LiveDiff)]);
}

/// The next request of the Window. Each read gives up after a millisecond so that the app runs
/// between two looks, until the deadline.
fn next_request(
    keeper: &mut std::os::unix::net::UnixStream,
    cx: &mut TestAppContext,
) -> (RequestId, Request) {
    let deadline = Instant::now() + common::DEADLINE;
    let mut first = [0u8; 1];
    loop {
        cx.run_until_parked();
        keeper
            .set_read_timeout(Some(Duration::from_millis(1)))
            .expect("a read timeout");
        match keeper.read(&mut first) {
            Ok(0) => panic!("the Window closed the connection, and a request was expected"),
            Ok(_) => break,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                assert!(Instant::now() < deadline, "the Window asked nothing");
            }
            Err(error) => panic!("a message of the Window: {error}"),
        }
    }
    keeper
        .set_read_timeout(Some(common::DEADLINE))
        .expect("a read timeout");
    match read_message(&mut Cursor::new(first).chain(&mut *keeper))
        .expect("a message of the Window")
    {
        Message::Request { id, request } => (id, request),
        other => panic!("the Window sent {other:?}, which is no request"),
    }
}

#[gpui_kit::test]
fn a_set_the_keeper_answers_with_a_reply_of_another_kind_has_the_list_asked_again(
    cx: &mut TestAppContext,
) {
    let link = cx.new(|_| KeeperLink::new());
    let list: Entity<SettingList> = cx.new(|cx| SettingList::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_deaf(&link, cx);
    let (first, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings);
    let kept = vec![form('2', FlowForm::LiveDiff)];
    let answered = Message::Reply {
        to: first,
        reply: Ok(Reply::Settings(kept.clone())),
    };
    write_message(&mut keeper, &answered).expect("answer the first question");
    wait_until(cx, "the first settings", |cx| {
        list.read_with(cx, |list, _| list.all() == kept)
    });

    list.update(cx, |list, cx| list.set(form('1', FlowForm::LiveDiff), cx));
    let (set, request) = next_request(&mut keeper, cx);
    assert_eq!(
        request,
        Request::SetSetting {
            setting: form('1', FlowForm::LiveDiff)
        }
    );
    let odd = Message::Reply {
        to: set,
        reply: Ok(Reply::Workspaces(Vec::new())),
    };
    write_message(&mut keeper, &odd).expect("answer with another kind of reply");
    let (second, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings, "asked again");
    let answered = Message::Reply {
        to: second,
        reply: Ok(Reply::Settings(kept.clone())),
    };
    write_message(&mut keeper, &answered).expect("answer the second question");
    wait_until(cx, "what the Keeper keeps", |cx| {
        list.read_with(cx, |list, _| list.all() == kept)
    });
    keeper.shutdown(Shutdown::Both).ok();
}

#[gpui_kit::test]
fn a_list_the_keeper_refuses_to_give_leaves_the_list_as_it_was(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let list: Entity<SettingList> = cx.new(|cx| SettingList::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_deaf(&link, cx);
    let (first, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings);
    let kept = vec![form('2', FlowForm::LiveDiff)];
    let answered = Message::Reply {
        to: first,
        reply: Ok(Reply::Settings(kept.clone())),
    };
    write_message(&mut keeper, &answered).expect("answer the first question");
    wait_until(cx, "the first settings", |cx| {
        list.read_with(cx, |list, _| list.all() == kept)
    });

    list.update(cx, |list, cx| list.set(form('1', FlowForm::LiveDiff), cx));
    let (set, request) = next_request(&mut keeper, cx);
    assert!(matches!(request, Request::SetSetting { .. }));
    let odd = Message::Reply {
        to: set,
        reply: Ok(Reply::Workspaces(Vec::new())),
    };
    write_message(&mut keeper, &odd).expect("answer with another kind of reply");
    let (second, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings, "asked again");
    let held_before = list.read_with(cx, |list, _| list.all().to_vec());
    assert_eq!(
        held_before,
        [form('2', FlowForm::LiveDiff), form('1', FlowForm::LiveDiff)]
    );

    let refused = Message::Reply {
        to: second,
        reply: Err(Refusal::Failed {
            reason: "the Store is not there".to_owned(),
        }),
    };
    write_message(&mut keeper, &refused).expect("refuse the second question");
    let barrier = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    let (asked, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::ListWorkspaces);
    let answered = Message::Reply {
        to: asked,
        reply: Ok(Reply::Workspaces(Vec::new())),
    };
    write_message(&mut keeper, &answered).expect("answer the barrier");
    answer(barrier, cx).expect("the barrier is answered");
    assert_eq!(
        list.read_with(cx, |list, _| list.all().to_vec()),
        held_before
    );
    keeper.shutdown(Shutdown::Both).ok();
}

#[gpui_kit::test]
fn of_two_answers_for_the_settings_only_the_one_to_the_last_question_counts_and_a_fact_on_the_way_asks_again(
    cx: &mut TestAppContext,
) {
    let link = cx.new(|_| KeeperLink::new());
    let list: Entity<SettingList> = cx.new(|cx| SettingList::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_deaf(&link, cx);

    let (first, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings);

    let now = vec![form('1', FlowForm::LiveDiff)];
    let told = Message::Fact(Fact::SettingChanged(now[0].clone()));
    write_message(&mut keeper, &told).expect("tell the fact");
    wait_until(cx, "the fact", |cx| {
        list.read_with(cx, |list, _| list.all() == now)
    });
    let (second, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings, "asked again");

    let stale = Message::Reply {
        to: first,
        reply: Ok(Reply::Settings(vec![form('1', FlowForm::Timeline)])),
    };
    write_message(&mut keeper, &stale).expect("answer late");
    let barrier = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    let (asked, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::ListWorkspaces);
    let answered = Message::Reply {
        to: asked,
        reply: Ok(Reply::Workspaces(Vec::new())),
    };
    write_message(&mut keeper, &answered).expect("answer the barrier");
    answer(barrier, cx).expect("the barrier is answered");
    assert_eq!(list.read_with(cx, |list, _| list.all().to_vec()), now);

    let last = vec![form('1', FlowForm::LiveDiff), form('2', FlowForm::LiveDiff)];
    let answered = Message::Reply {
        to: second,
        reply: Ok(Reply::Settings(last.clone())),
    };
    write_message(&mut keeper, &answered).expect("answer");
    wait_until(cx, "the answer to the last question", |cx| {
        list.read_with(cx, |list, _| list.all() == last)
    });
    // Ended, so that the Window's thread is not left on a socket nobody reads.
    keeper.shutdown(Shutdown::Both).ok();
}

#[gpui_kit::test]
fn an_answer_for_the_settings_asked_before_a_set_does_not_replace_the_list_the_set_changed(
    cx: &mut TestAppContext,
) {
    let link = cx.new(|_| KeeperLink::new());
    let list: Entity<SettingList> = cx.new(|cx| SettingList::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach_deaf(&link, cx);
    let (first, request) = next_request(&mut keeper, cx);
    assert_eq!(request, Request::GetSettings);

    let chosen = form('1', FlowForm::LiveDiff);
    list.update(cx, |list, cx| list.set(chosen.clone(), cx));

    let other = form('2', FlowForm::LiveDiff);
    let stale = Message::Reply {
        to: first,
        reply: Ok(Reply::Settings(vec![other.clone()])),
    };
    write_message(&mut keeper, &stale).expect("answer late");
    let barrier = link.update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
    let mut asked_again = None;
    let mut set_heard = false;
    let barrier_id = loop {
        let (asked, request) = next_request(&mut keeper, cx);
        match request {
            Request::ListWorkspaces => break asked,
            Request::SetSetting { setting } => {
                assert_eq!(setting, chosen);
                set_heard = true;
                let kept = Message::Reply {
                    to: asked,
                    reply: Ok(Reply::Done),
                };
                write_message(&mut keeper, &kept).expect("answer the set");
            }
            Request::GetSettings => {
                assert!(
                    set_heard,
                    "the list is asked for again before the Keeper heard the set"
                );
                asked_again = Some(asked);
            }
            _ => {}
        }
    };
    let answered = Message::Reply {
        to: barrier_id,
        reply: Ok(Reply::Workspaces(Vec::new())),
    };
    write_message(&mut keeper, &answered).expect("answer the barrier");
    answer(barrier, cx).expect("the barrier is answered");

    assert_eq!(
        list.read_with(cx, |list, _| list.all().to_vec()),
        std::slice::from_ref(&chosen),
        "the answer that knew nothing of the set was held"
    );
    assert_eq!(
        list.read_with(cx, |list, _| list.flow_form(&id('1'))),
        FlowForm::LiveDiff
    );

    let asked_again = asked_again.expect("the list is asked for again after the set");
    let kept_both = vec![other, chosen];
    let answered = Message::Reply {
        to: asked_again,
        reply: Ok(Reply::Settings(kept_both.clone())),
    };
    write_message(&mut keeper, &answered).expect("answer the new question");
    wait_until(cx, "the answer to the question asked after the set", |cx| {
        list.read_with(cx, |list, _| list.all() == kept_both)
    });
    keeper.shutdown(Shutdown::Both).ok();
}
