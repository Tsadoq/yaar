//! Component test: the shared model of the questions for leave, through what the terminal
//! pane uses of it. The test plays the Keeper on the other end of a socket pair: it tells
//! the model facts, answers what the model asks, and reads back through the model.
//!
//! A negative clause ("asks nothing") is read after a barrier: the next request the Keeper
//! end reads is the one the test caused after it, so nothing came between.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{AppContext, Entity, Task, TestAppContext};
use yaar_frame::{AskError, ConsentList, KeeperLink};
use yaar_link::{Consent, ConsentAnswer, Fact, Refusal, Reply, Request};

use common::{KeeperEnd, wait_until};

/// A consent of `agent` with the words `question`, answered as `answer`.
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

/// A list linked to a Keeper the test plays, which has told it nothing yet.
fn linked(cx: &mut TestAppContext) -> (Entity<ConsentList>, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let keeper = KeeperEnd::attach(&link, cx);
    let list = cx.new(|cx| ConsentList::new(link, cx));
    (list, keeper)
}

/// What the model shows as questions, by agent.
fn asked(list: &Entity<ConsentList>, cx: &TestAppContext) -> Vec<String> {
    list.read_with(cx, |list, _| {
        list.asked().into_iter().map(|c| c.agent).collect()
    })
}

/// Tells the model what a Keeper tells at a connect, and waits until the model holds it.
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

/// The answer a task resolves to, once it does, without waiting for it: the test has to
/// answer the Keeper's requests while it is on its way.
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

    // Barrier: had the model asked anything at the connect, or when it was told, that
    // question would be read before the answer this call sends.
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

    // A list told replaces the one before whole: what it leaves out is gone.
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

    // Nothing for the one answered already, and no question for the list: the questions
    // shown are known.
    let (first, request) = keeper.request(cx);
    assert_eq!(request, answering("a", ConsentAnswer::Yes));
    let (second, request) = keeper.request(cx);
    assert_eq!(request, answering("c", ConsentAnswer::Yes));

    // The Keeper tells another list while the answers are on their way; the answers that
    // follow replace it, the last as the Keeper's last word.
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
    // Each listed Adapter is answered, whatever it said before, one after the other.
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

    // Barrier: the next request is the question of the next answer, so nothing was sent
    // between.
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

    // The first is refused and the second goes through: the list the Keeper answers the
    // second with is taken though the task resolves to the refusal.
    let refusal = Refusal::Failed {
        reason: "no such agent".to_owned(),
    };
    keeper.reply(first, Err(refusal.clone()));
    // The refusal is heard before the second answer comes, as it may be: a fact the Keeper
    // tells after it is taken after it, so the task has seen the refusal by then. The task
    // waits for the second answer all the same.
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
