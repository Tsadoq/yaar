mod common;

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{AppContext, Entity, Task, TestAppContext};
use yaar_frame::{AskError, ConsentList, KeeperLink};
use yaar_link::{Consent, ConsentAnswer, Fact, Refusal, Reply, Request};

use common::{KeeperEnd, wait_until};

fn consent(agent: &str, answer: ConsentAnswer) -> Consent {
    Consent {
        agent: agent.to_owned(),
        question: format!("May the app change the settings of {agent}?"),
        answer,
        problem: None,
    }
}

fn failed(agent: &str, answer: ConsentAnswer, problem: &str) -> Consent {
    Consent {
        problem: Some(problem.to_owned()),
        ..consent(agent, answer)
    }
}

fn answering(agent: &str, answer: ConsentAnswer) -> Request {
    Request::AnswerConsent {
        agent: agent.to_owned(),
        answer,
    }
}

fn linked(cx: &mut TestAppContext) -> (Entity<ConsentList>, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let keeper = KeeperEnd::attach(&link, cx);
    let list = cx.new(|cx| ConsentList::new(link, cx));
    (list, keeper)
}

fn asked(list: &Entity<ConsentList>, cx: &TestAppContext) -> Vec<String> {
    list.read_with(cx, |list, _| {
        list.asked().into_iter().map(|c| c.agent).collect()
    })
}

fn tell(
    list: &Entity<ConsentList>,
    keeper: &mut KeeperEnd,
    consents: Vec<Consent>,
    cx: &mut TestAppContext,
) {
    let wanted: Vec<String> = consents
        .iter()
        .filter(|c| c.answer == ConsentAnswer::NotAsked || c.problem.is_some())
        .map(|c| c.agent.clone())
        .collect();
    keeper.tell(Fact::ConsentsChanged(consents));
    let list = list.clone();
    wait_until(cx, "the model to take what was told", move |cx| {
        asked(&list, cx) == wanted
    });
}

type Slot<T> = Rc<RefCell<Option<T>>>;

fn start<T: 'static>(task: Task<T>, cx: &mut TestAppContext) -> Slot<T> {
    let slot = Rc::new(RefCell::new(None));
    cx.foreground_executor()
        .spawn({
            let slot = slot.clone();
            async move {
                let answer = task.await;
                *slot.borrow_mut() = Some(answer);
            }
        })
        .detach();
    slot
}

fn finished<T: 'static>(slot: &Slot<T>, cx: &mut TestAppContext) -> T {
    let waiting = slot.clone();
    wait_until(cx, "the answer", move |_| waiting.borrow().is_some());
    slot.take().expect("the answer")
}

#[gpui_kit::test]
fn the_model_asks_nothing_at_a_connect_and_holds_what_the_keeper_tells(cx: &mut TestAppContext) {
    let (list, mut keeper) = linked(cx);
    assert_eq!(asked(&list, cx), Vec::<String>::new());

    tell(
        &list,
        &mut keeper,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        cx,
    );
    let words = list.read_with(cx, |list, _| list.asked());
    assert_eq!(words, [consent("pi", ConsentAnswer::NotAsked)]);

    // Barrier: had the model asked anything at the connect or when told, that question would be
    // read first.
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::No, cx));
    let sent = start(sent, cx);
    let (id, request) = keeper.request(cx);
    assert_eq!(request, answering("pi", ConsentAnswer::No));
    keeper.reply(
        id,
        Ok(Reply::Consents(vec![consent("pi", ConsentAnswer::No)])),
    );
    assert!(matches!(finished(&sent, cx), Ok(true)));
}

#[gpui_kit::test]
fn the_questions_are_the_unanswered_and_the_failed_ones_in_the_order_of_the_adapters(
    cx: &mut TestAppContext,
) {
    let (list, mut keeper) = linked(cx);
    tell(
        &list,
        &mut keeper,
        vec![
            consent("a", ConsentAnswer::NotAsked),
            consent("b", ConsentAnswer::Yes),
            failed("c", ConsentAnswer::No, "the file is not JSON"),
            consent("d", ConsentAnswer::No),
            failed("e", ConsentAnswer::NotAsked, "could not be written"),
        ],
        cx,
    );
    assert_eq!(asked(&list, cx), ["a", "c", "e"]);
    let problems: Vec<Option<String>> = list.read_with(cx, |list, _| {
        list.asked().into_iter().map(|c| c.problem).collect()
    });
    assert_eq!(
        problems,
        [
            None,
            Some("the file is not JSON".to_owned()),
            Some("could not be written".to_owned())
        ]
    );

    tell(
        &list,
        &mut keeper,
        vec![consent("a", ConsentAnswer::Yes)],
        cx,
    );
    assert_eq!(asked(&list, cx), Vec::<String>::new());
}

#[gpui_kit::test]
fn an_answer_with_questions_shown_goes_for_each_of_them_and_the_keepers_list_replaces_what_was_told(
    cx: &mut TestAppContext,
) {
    let (list, mut keeper) = linked(cx);
    tell(
        &list,
        &mut keeper,
        vec![
            consent("a", ConsentAnswer::NotAsked),
            consent("b", ConsentAnswer::Yes),
            consent("c", ConsentAnswer::NotAsked),
        ],
        cx,
    );
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::Yes, cx));
    let sent = start(sent, cx);

    let (first, request) = keeper.request(cx);
    assert_eq!(request, answering("a", ConsentAnswer::Yes));
    let (second, request) = keeper.request(cx);
    assert_eq!(request, answering("c", ConsentAnswer::Yes));

    keeper.tell(Fact::ConsentsChanged(vec![
        consent("a", ConsentAnswer::NotAsked),
        consent("b", ConsentAnswer::NotAsked),
        consent("c", ConsentAnswer::NotAsked),
    ]));
    keeper.reply(
        first,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::Yes),
            consent("c", ConsentAnswer::NotAsked),
        ])),
    );
    keeper.reply(
        second,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::Yes),
            consent("c", ConsentAnswer::Yes),
        ])),
    );
    assert!(matches!(finished(&sent, cx), Ok(true)));
    assert_eq!(asked(&list, cx), Vec::<String>::new());
}

#[gpui_kit::test]
fn an_answer_with_none_shown_asks_the_list_first_and_answers_each_listed_adapter(
    cx: &mut TestAppContext,
) {
    let (list, mut keeper) = linked(cx);
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::Yes, cx));
    let sent = start(sent, cx);

    let asking = keeper.expect(Request::ListConsents, cx);
    keeper.reply(
        asking,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::No),
            consent("b", ConsentAnswer::Yes),
        ])),
    );
    let first = keeper.expect(answering("a", ConsentAnswer::Yes), cx);
    keeper.reply(
        first,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::Yes),
        ])),
    );
    let second = keeper.expect(answering("b", ConsentAnswer::Yes), cx);
    keeper.reply(
        second,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::Yes),
        ])),
    );
    assert!(matches!(finished(&sent, cx), Ok(true)));
    assert_eq!(asked(&list, cx), Vec::<String>::new());
}

#[gpui_kit::test]
fn an_answer_when_the_keeper_lists_none_says_so_and_sends_nothing_more(cx: &mut TestAppContext) {
    let (list, mut keeper) = linked(cx);
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::No, cx));
    let sent = start(sent, cx);
    let asking = keeper.expect(Request::ListConsents, cx);
    keeper.reply(asking, Ok(Reply::Consents(Vec::new())));
    assert!(matches!(finished(&sent, cx), Ok(false)));

    let again = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::No, cx));
    let again = start(again, cx);
    let asking = keeper.expect(Request::ListConsents, cx);
    keeper.reply(asking, Ok(Reply::Consents(Vec::new())));
    assert!(matches!(finished(&again, cx), Ok(false)));
}

#[gpui_kit::test]
fn a_refused_answer_is_the_tasks_refusal_and_the_list_stays_as_it_was(cx: &mut TestAppContext) {
    let (list, mut keeper) = linked(cx);
    tell(
        &list,
        &mut keeper,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        cx,
    );
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::Yes, cx));
    let sent = start(sent, cx);
    let (id, request) = keeper.request(cx);
    assert_eq!(request, answering("pi", ConsentAnswer::Yes));
    let refusal = Refusal::Failed {
        reason: "no such agent".to_owned(),
    };
    keeper.reply(id, Err(refusal.clone()));
    assert!(matches!(
        finished(&sent, cx),
        Err(AskError::Refused(refused)) if refused == refusal
    ));
    assert_eq!(asked(&list, cx), ["pi"]);
}

#[gpui_kit::test]
fn a_refusal_of_the_first_answer_still_takes_the_list_of_the_others(cx: &mut TestAppContext) {
    let (list, mut keeper) = linked(cx);
    tell(
        &list,
        &mut keeper,
        vec![
            consent("a", ConsentAnswer::NotAsked),
            consent("b", ConsentAnswer::NotAsked),
        ],
        cx,
    );
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::Yes, cx));
    let sent = start(sent, cx);
    let (first, request) = keeper.request(cx);
    assert_eq!(request, answering("a", ConsentAnswer::Yes));
    let (second, request) = keeper.request(cx);
    assert_eq!(request, answering("b", ConsentAnswer::Yes));

    let refusal = Refusal::Failed {
        reason: "no such agent".to_owned(),
    };
    keeper.reply(first, Err(refusal.clone()));
    // The refusal is heard before the second answer comes, as it may be. A fact told after it is
    // taken after it, so once the telling below is taken the task has seen the refusal, and it
    // still waits for the second answer.
    tell(
        &list,
        &mut keeper,
        vec![
            consent("a", ConsentAnswer::NotAsked),
            consent("b", ConsentAnswer::NotAsked),
            consent("c", ConsentAnswer::NotAsked),
        ],
        cx,
    );
    cx.run_until_parked();
    assert!(
        sent.borrow().is_none(),
        "the task resolved before the second answer came"
    );
    keeper.reply(
        second,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::NotAsked),
            consent("b", ConsentAnswer::Yes),
        ])),
    );
    assert!(matches!(
        finished(&sent, cx),
        Err(AskError::Refused(refused)) if refused == refusal
    ));
    assert_eq!(asked(&list, cx), ["a"]);
}

#[gpui_kit::test]
fn an_answer_when_every_question_held_is_answered_already_asks_the_list_first(
    cx: &mut TestAppContext,
) {
    let (list, mut keeper) = linked(cx);
    // A question is shown first, so that the empty list after the next fact shows the model took
    // that fact: an empty list is also what a model that heard nothing gives.
    tell(
        &list,
        &mut keeper,
        vec![consent("b", ConsentAnswer::NotAsked)],
        cx,
    );
    // The model holds an Adapter whose question was answered, so it shows no question. The answer
    // is for the Adapters the Keeper lists, not for the questions the model shows.
    tell(
        &list,
        &mut keeper,
        vec![consent("a", ConsentAnswer::No)],
        cx,
    );
    assert_eq!(asked(&list, cx), Vec::<String>::new());
    let sent = list.update(cx, |list, cx| list.answer_all(ConsentAnswer::Yes, cx));
    let sent = start(sent, cx);

    let asking = keeper.expect(Request::ListConsents, cx);
    keeper.reply(
        asking,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::No),
            consent("b", ConsentAnswer::NotAsked),
        ])),
    );
    let first = keeper.expect(answering("a", ConsentAnswer::Yes), cx);
    keeper.reply(
        first,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::NotAsked),
        ])),
    );
    let second = keeper.expect(answering("b", ConsentAnswer::Yes), cx);
    keeper.reply(
        second,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::Yes),
        ])),
    );
    assert!(matches!(finished(&sent, cx), Ok(true)));
    assert_eq!(asked(&list, cx), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_refresh_asks_for_the_whole_list_and_takes_it_and_a_refusal_leaves_the_list_as_it_was(
    cx: &mut TestAppContext,
) {
    let (list, mut keeper) = linked(cx);
    let refreshed = list.update(cx, |list, cx| list.refresh(cx));
    let refreshed = start(refreshed, cx);
    let asking = keeper.expect(Request::ListConsents, cx);
    keeper.reply(
        asking,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::No),
        ])),
    );
    assert!(matches!(finished(&refreshed, cx), Ok(())));
    assert_eq!(
        list.read_with(cx, |list, _| list.all().to_vec()),
        [
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::No)
        ]
    );
    assert_eq!(asked(&list, cx), Vec::<String>::new());

    let refused = list.update(cx, |list, cx| list.refresh(cx));
    let refused = start(refused, cx);
    let asking = keeper.expect(Request::ListConsents, cx);
    keeper.reply(
        asking,
        Err(Refusal::Failed {
            reason: "no list".to_owned(),
        }),
    );
    assert!(matches!(
        finished(&refused, cx),
        Err(AskError::Refused(Refusal::Failed { .. }))
    ));
    assert_eq!(list.read_with(cx, |list, _| list.all().len()), 2);
}

#[gpui_kit::test]
fn the_list_holds_every_adapter_the_keeper_told_answered_or_not(cx: &mut TestAppContext) {
    let (list, mut keeper) = linked(cx);
    assert_eq!(list.read_with(cx, |list, _| list.all().to_vec()), []);

    let told = vec![
        consent("a", ConsentAnswer::NotAsked),
        consent("b", ConsentAnswer::Yes),
        consent("c", ConsentAnswer::No),
    ];
    tell(&list, &mut keeper, told.clone(), cx);

    assert_eq!(asked(&list, cx), ["a"]);
    assert_eq!(list.read_with(cx, |list, _| list.all().to_vec()), told);
}

#[gpui_kit::test]
fn an_answer_for_one_adapter_goes_for_that_adapter_alone_and_the_keepers_list_replaces_the_held(
    cx: &mut TestAppContext,
) {
    let (list, mut keeper) = linked(cx);
    tell(
        &list,
        &mut keeper,
        vec![
            consent("a", ConsentAnswer::Yes),
            consent("b", ConsentAnswer::NotAsked),
        ],
        cx,
    );

    let sent = list.update(cx, |list, cx| {
        list.answer("a".to_owned(), ConsentAnswer::No, cx)
    });
    let sent = start(sent, cx);
    let (id, request) = keeper.request(cx);
    assert_eq!(request, answering("a", ConsentAnswer::No));
    keeper.reply(
        id,
        Ok(Reply::Consents(vec![
            consent("a", ConsentAnswer::No),
            consent("b", ConsentAnswer::NotAsked),
        ])),
    );
    assert!(matches!(finished(&sent, cx), Ok(())));
    assert_eq!(
        list.read_with(cx, |list, _| list.all().to_vec()),
        [
            consent("a", ConsentAnswer::No),
            consent("b", ConsentAnswer::NotAsked)
        ]
    );

    let sent = list.update(cx, |list, cx| {
        list.answer("b".to_owned(), ConsentAnswer::Yes, cx)
    });
    let sent = start(sent, cx);
    let (id, request) = keeper.request(cx);
    assert_eq!(request, answering("b", ConsentAnswer::Yes));
    keeper.reply(
        id,
        Err(Refusal::Failed {
            reason: "no such adapter".to_owned(),
        }),
    );
    assert!(matches!(
        finished(&sent, cx),
        Err(AskError::Refused(Refusal::Failed { .. }))
    ));
    assert_eq!(
        list.read_with(cx, |list, _| list.all()[1].answer),
        ConsentAnswer::NotAsked
    );
}
