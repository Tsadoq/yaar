//! Component test (CONSENT-06): the question an Adapter asks before it writes into the
//! user's own settings of its CLI, in the terminal pane, on GPUI's test platform with no
//! window on any screen. The test plays the Keeper on the other end of a socket pair: it
//! tells the question, reads the answer the pane sends, and tells what the Keeper would
//! tell next; the pane is read back through the model it shows and the elements it draws.
//!
//! The words of a question are the test's data: the pane names no CLI. A negative clause
//! ("nothing more is sent") is read after a barrier: the next request the Keeper end reads
//! is one the test caused after it, so nothing came between.

mod common;

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_link::{Consent, ConsentAnswer, Fact, Program, Refusal, Reply, Request};
use yaar_terminal_pane::{
    InstallAgentExtensions, KeepAgentExtensionsOut, NOTHING_TO_INSTALL, NewTerminal,
};

use common::{Panes, click_in, drawn, key_of, open_panes, terminal, wait_drawn, wait_until};

/// A question of the Adapter `agent` in words of the test's own.
fn consent(agent: &str, answer: ConsentAnswer) -> Consent {
    Consent {
        agent: agent.to_owned(),
        question: format!("May the app add its extension to the list in the settings of {agent}?"),
        answer,
        problem: None,
    }
}

fn answering(agent: &str, answer: ConsentAnswer) -> Request {
    Request::AnswerConsent {
        agent: agent.to_owned(),
        answer,
    }
}

/// Tells the Window the questions, and waits until the pane shows them: the ones not
/// answered, and the ones whose settings could not be changed.
fn tell(panes: &Panes, consents: Vec<Consent>, cx: &mut VisualTestContext) {
    let wanted: Vec<Consent> = consents
        .iter()
        .filter(|c| c.answer == ConsentAnswer::NotAsked || c.problem.is_some())
        .cloned()
        .collect();
    panes.keeper.tell(Fact::ConsentsChanged(consents));
    let pane = panes.pane.clone();
    wait_drawn(cx, "the pane to take the list", move |cx| {
        pane.read_with(cx, |pane, cx| pane.questions(cx)) == wanted
            && drawn("terminal-consent", cx).is_some() == !wanted.is_empty()
    });
}

/// Answers the next request, which has to be `wanted`, with the list `consents`.
fn answer(panes: &mut Panes, wanted: Request, consents: Vec<Consent>, cx: &mut VisualTestContext) {
    let (id, request) = panes.keeper.request(cx);
    assert_eq!(request, wanted);
    panes.keeper.reply(id, Ok(Reply::Consents(consents)));
}

fn gone(cx: &mut VisualTestContext) {
    wait_drawn(cx, "the line to go", |cx| {
        drawn("terminal-consent", cx).is_none()
    });
    for selector in [
        "terminal-consent-question-0",
        "terminal-consent-yes",
        "terminal-consent-no",
    ] {
        assert!(drawn(selector, cx).is_none(), "{selector} is still drawn");
    }
}

/// After a barrier: the next request is the one of Cmd-T, so that nothing was sent
/// before it.
fn nothing_else_was_sent(panes: &mut Panes, cx: &mut VisualTestContext) {
    panes.press_in_the_app::<NewTerminal>(cx);
    let (_, request) = panes.keeper.request(cx);
    assert!(
        matches!(
            request,
            Request::OpenTerminal {
                program: Program::Shell,
                ..
            }
        ),
        "the Window sent {request:?} before the barrier"
    );
}

fn questions(panes: &Panes, cx: &mut VisualTestContext) -> Vec<Consent> {
    panes.pane.read_with(cx, |pane, cx| pane.questions(cx))
}

#[gpui_kit::test]
fn each_question_the_keeper_tells_shows_in_the_adapters_words_with_two_controls_above_the_terminals(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let t = terminal('a');
    panes.open_terminal(&t, &mut cx);
    assert!(drawn("terminal-consent", &mut cx).is_none());
    assert_eq!(questions(&panes, &mut cx), []);

    let told = vec![
        consent("pi", ConsentAnswer::NotAsked),
        consent("other", ConsentAnswer::NotAsked),
    ];
    tell(&panes, told.clone(), &mut cx);

    // The words as told, each question in the order of the Adapters.
    assert_eq!(questions(&panes, &mut cx), told);
    let line = drawn("terminal-consent", &mut cx).expect("the line");
    let first = drawn("terminal-consent-question-0", &mut cx).expect("the first question");
    let second = drawn("terminal-consent-question-1", &mut cx).expect("the second question");
    assert!(drawn("terminal-consent-question-2", &mut cx).is_none());
    assert!(first.bottom() <= second.top());
    // The controls come after the last question and answer every one.
    let yes = drawn("terminal-consent-yes", &mut cx).expect("the yes control");
    let no = drawn("terminal-consent-no", &mut cx).expect("the no control");
    assert!(second.bottom() <= yes.top() && second.bottom() <= no.top());
    assert!(yes.right() <= no.left());
    // No problem to show where none went wrong.
    assert!(drawn("terminal-consent-problem-0", &mut cx).is_none());
    assert!(drawn("terminal-consent-problem-1", &mut cx).is_none());

    // The line lies in the pane's top row: every element of it is inside, and it ends at
    // or above where the terminal begins.
    for part in [first, second, yes, no] {
        assert!(line.contains(&part.origin) && line.contains(&part.bottom_right()));
    }
    let slot = drawn("terminal-slot-0", &mut cx).expect("the terminal's slot");
    assert!(line.top() <= first.top());
    assert!(line.bottom() <= slot.top());
}

/// The width of what is drawn for `selector`, which on the test platform is the number of
/// its characters times the width of one.
fn width(selector: &str, cx: &mut VisualTestContext) -> f32 {
    let bounds = drawn(selector, cx).unwrap_or_else(|| panic!("{selector} is not drawn"));
    f32::from(bounds.size.width)
}

#[gpui_kit::test]
fn the_text_drawn_for_a_question_and_its_problem_is_the_words_told_and_not_the_name_of_the_adapter(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx) = open_panes(cx);
    // No spaces: the test platform measures a space differently from a letter. The words
    // are told twice with one length each; the Adapter's name is the same both times, so
    // only the words can make the drawn text change its width.
    let short = "Shortwords";
    let long = "Somewhatlongerwordsoftheadapterhere";
    let told = |words: &str, problem: &str| {
        let mut told = consent("pi", ConsentAnswer::NotAsked);
        told.question = words.to_owned();
        told.problem = Some(problem.to_owned());
        vec![told]
    };
    let ratio = long.chars().count() as f32 / short.chars().count() as f32;

    tell(&panes, told(short, short), &mut cx);
    let question_short = width("terminal-consent-question-0", &mut cx);
    let problem_short = width("terminal-consent-problem-0", &mut cx);
    tell(&panes, told(long, long), &mut cx);
    let question_long = width("terminal-consent-question-0", &mut cx);
    let problem_long = width("terminal-consent-problem-0", &mut cx);

    // A width is rounded to a pixel, hence the margin.
    assert!(question_short > 0.0 && problem_short > 0.0);
    assert!(
        (question_long / question_short - ratio).abs() < 0.1,
        "the question drawn is {ratio} times as wide for words that long: \
         {question_short} and {question_long}"
    );
    assert!(
        (problem_long / problem_short - ratio).abs() < 0.1,
        "the problem drawn is {ratio} times as wide for words that long: \
         {problem_short} and {problem_long}"
    );
}

#[gpui_kit::test]
fn a_problem_the_keeper_tells_is_shown_under_its_question(cx: &mut TestAppContext) {
    let (panes, mut cx) = open_panes(cx);
    let mut told = consent("pi", ConsentAnswer::NotAsked);
    told.problem = Some("the settings are not JSON, so the app left them alone".to_owned());
    tell(
        &panes,
        vec![told.clone(), consent("other", ConsentAnswer::NotAsked)],
        &mut cx,
    );

    assert_eq!(questions(&panes, &mut cx)[0], told);
    let question = drawn("terminal-consent-question-0", &mut cx).expect("the question");
    let problem = drawn("terminal-consent-problem-0", &mut cx).expect("the problem");
    let next = drawn("terminal-consent-question-1", &mut cx).expect("the second question");
    assert!(question.bottom() <= problem.top());
    assert!(problem.bottom() <= next.top());
    assert!(drawn("terminal-consent-problem-1", &mut cx).is_none());

    // A yes whose write failed is asked again with its problem, though answered.
    let mut failed = consent("pi", ConsentAnswer::Yes);
    failed.problem = Some("the settings could not be written".to_owned());
    tell(&panes, vec![failed.clone()], &mut cx);
    assert_eq!(questions(&panes, &mut cx), [failed]);
    assert!(drawn("terminal-consent-problem-0", &mut cx).is_some());
}

#[gpui_kit::test]
fn the_line_blocks_nothing_and_the_terminals_are_resized_as_it_comes_and_goes(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let (t1, t2) = (terminal('a'), terminal('b'));
    panes.open_terminal(&t1, &mut cx);
    // The grid the terminal fits in the whole pane, which its first resize tells.
    wait_until(&mut cx, "the terminal to fit the pane", |_| {
        panes.keeper.resizes(&t1).len() == 1
    });
    let whole = panes.keeper.resizes(&t1)[0];

    // It comes: the terminal has the room under the line, which is less.
    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );
    wait_until(&mut cx, "the terminal to be resized under the line", |_| {
        panes.keeper.resizes(&t1).len() == 2
    });
    let under = panes.keeper.resizes(&t1)[1];
    assert_eq!(under.columns, whole.columns);
    assert!(under.rows < whole.rows);

    // It blocks nothing: a terminal opens while it shows, and is laid out in the room
    // under the line.
    panes.open_terminal(&t2, &mut cx);
    assert_eq!(panes.grid(&t2), under);
    assert!(drawn("terminal-consent", &mut cx).is_some());

    // It goes: the terminal on screen has the whole pane again.
    panes.keeper.tell(Fact::ConsentsChanged(vec![consent(
        "pi",
        ConsentAnswer::Yes,
    )]));
    gone(&mut cx);
    wait_until(&mut cx, "the terminal to be resized back", |_| {
        panes.keeper.resizes(&t2).last() == Some(&whole)
    });
}

#[gpui_kit::test]
fn a_click_on_a_control_sends_the_answer_and_the_line_goes_once_the_answer_is_told(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );

    // Install: the next request is the answer, with no question for the list before it.
    click_in("terminal-consent-yes", &mut cx);
    // The line stays until the answer is told: the Keeper has not said yes yet.
    assert!(drawn("terminal-consent", &mut cx).is_some());
    answer(
        &mut panes,
        answering("pi", ConsentAnswer::Yes),
        vec![consent("pi", ConsentAnswer::Yes)],
        &mut cx,
    );
    gone(&mut cx);
    nothing_else_was_sent(&mut panes, &mut cx);
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.notice()), None);

    // No, for the question told again.
    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );
    click_in("terminal-consent-no", &mut cx);
    answer(
        &mut panes,
        answering("pi", ConsentAnswer::No),
        vec![consent("pi", ConsentAnswer::No)],
        &mut cx,
    );
    gone(&mut cx);
    nothing_else_was_sent(&mut panes, &mut cx);
}

#[gpui_kit::test]
fn one_answer_goes_for_every_question_shown(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    tell(
        &panes,
        vec![
            consent("pi", ConsentAnswer::NotAsked),
            consent("other", ConsentAnswer::NotAsked),
        ],
        &mut cx,
    );
    click_in("terminal-consent-no", &mut cx);
    let (first, request) = panes.keeper.request(&mut cx);
    assert_eq!(request, answering("pi", ConsentAnswer::No));
    let (second, request) = panes.keeper.request(&mut cx);
    assert_eq!(request, answering("other", ConsentAnswer::No));
    panes.keeper.reply(
        first,
        Ok(Reply::Consents(vec![
            consent("pi", ConsentAnswer::No),
            consent("other", ConsentAnswer::NotAsked),
        ])),
    );
    // The line holds the question still unanswered until the last answer is told.
    let pane = panes.pane.clone();
    wait_until(&mut cx, "the first answer to be taken", |cx| {
        pane.read_with(cx, |pane, cx| pane.questions(cx).len()) == 1
    });
    assert!(drawn("terminal-consent", &mut cx).is_some());
    assert_eq!(questions(&panes, &mut cx)[0].agent, "other");
    panes.keeper.reply(
        second,
        Ok(Reply::Consents(vec![
            consent("pi", ConsentAnswer::No),
            consent("other", ConsentAnswer::No),
        ])),
    );
    gone(&mut cx);
    nothing_else_was_sent(&mut panes, &mut cx);
}

#[gpui_kit::test]
fn the_two_keys_answer_with_a_question_shown_from_outside_the_terminal_and_on_macos_from_inside(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let t = terminal('a');
    panes.open_terminal(&t, &mut cx);

    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );
    panes.press_in_the_app::<KeepAgentExtensionsOut>(&mut cx);
    answer(
        &mut panes,
        answering("pi", ConsentAnswer::No),
        vec![consent("pi", ConsentAnswer::No)],
        &mut cx,
    );
    gone(&mut cx);

    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );
    panes.press_in_the_app::<InstallAgentExtensions>(&mut cx);
    answer(
        &mut panes,
        answering("pi", ConsentAnswer::Yes),
        vec![consent("pi", ConsentAnswer::Yes)],
        &mut cx,
    );
    gone(&mut cx);
    nothing_else_was_sent(&mut panes, &mut cx);

    // The keys are kept on macOS, where they work with the focus in the terminal too.
    if cfg!(target_os = "macos") {
        tell(
            &panes,
            vec![consent("pi", ConsentAnswer::NotAsked)],
            &mut cx,
        );
        panes.click_slot(0, &mut cx);
        assert_eq!(panes.focused_terminal(&mut cx), Some(t.clone()));
        cx.simulate_keystrokes(key_of::<KeepAgentExtensionsOut>());
        answer(
            &mut panes,
            answering("pi", ConsentAnswer::No),
            vec![consent("pi", ConsentAnswer::No)],
            &mut cx,
        );
        gone(&mut cx);

        tell(
            &panes,
            vec![consent("pi", ConsentAnswer::NotAsked)],
            &mut cx,
        );
        panes.click_slot(0, &mut cx);
        assert_eq!(panes.focused_terminal(&mut cx), Some(t.clone()));
        cx.simulate_keystrokes(key_of::<InstallAgentExtensions>());
        answer(
            &mut panes,
            answering("pi", ConsentAnswer::Yes),
            vec![consent("pi", ConsentAnswer::Yes)],
            &mut cx,
        );
        gone(&mut cx);
    }
}

#[gpui_kit::test]
fn a_list_the_keeper_tells_without_a_question_takes_the_line_away(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );

    // Another Window answered: nothing of this one was sent for it.
    panes.keeper.tell(Fact::ConsentsChanged(vec![consent(
        "pi",
        ConsentAnswer::Yes,
    )]));
    gone(&mut cx);
    assert_eq!(questions(&panes, &mut cx), []);
    nothing_else_was_sent(&mut panes, &mut cx);
}

#[gpui_kit::test]
fn with_no_question_shown_the_keys_ask_the_list_and_answer_each_adapter_it_names(
    cx: &mut TestAppContext,
) {
    let (mut panes, mut cx) = open_panes(cx);
    let t = terminal('a');
    panes.open_terminal(&t, &mut cx);
    // Answered, not asked: no line.
    tell(&panes, vec![consent("pi", ConsentAnswer::No)], &mut cx);

    // A yes after a no: the list first, then the answer for each Adapter it names.
    panes.press_in_the_app::<InstallAgentExtensions>(&mut cx);
    answer(
        &mut panes,
        Request::ListConsents,
        vec![
            consent("pi", ConsentAnswer::No),
            consent("other", ConsentAnswer::Yes),
        ],
        &mut cx,
    );
    answer(
        &mut panes,
        answering("pi", ConsentAnswer::Yes),
        vec![
            consent("pi", ConsentAnswer::Yes),
            consent("other", ConsentAnswer::Yes),
        ],
        &mut cx,
    );
    answer(
        &mut panes,
        answering("other", ConsentAnswer::Yes),
        vec![
            consent("pi", ConsentAnswer::Yes),
            consent("other", ConsentAnswer::Yes),
        ],
        &mut cx,
    );
    nothing_else_was_sent(&mut panes, &mut cx);
    assert!(drawn("terminal-consent", &mut cx).is_none());
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.notice()), None);

    // A no after a yes, with the focus in the terminal on macOS: the same, the other way.
    let keep = || {
        vec![
            consent("pi", ConsentAnswer::No),
            consent("other", ConsentAnswer::No),
        ]
    };
    if cfg!(target_os = "macos") {
        panes.click_slot(0, &mut cx);
        assert_eq!(panes.focused_terminal(&mut cx), Some(t.clone()));
        cx.simulate_keystrokes(key_of::<KeepAgentExtensionsOut>());
    } else {
        panes.press_in_the_app::<KeepAgentExtensionsOut>(&mut cx);
    }
    answer(&mut panes, Request::ListConsents, keep(), &mut cx);
    answer(
        &mut panes,
        answering("pi", ConsentAnswer::No),
        keep(),
        &mut cx,
    );
    answer(
        &mut panes,
        answering("other", ConsentAnswer::No),
        keep(),
        &mut cx,
    );
    nothing_else_was_sent(&mut panes, &mut cx);
}

#[gpui_kit::test]
fn with_no_adapter_listed_the_notice_says_so_and_nothing_more_is_sent(cx: &mut TestAppContext) {
    let (mut panes, mut cx) = open_panes(cx);
    assert_eq!(panes.pane.read_with(&cx, |pane, _| pane.notice()), None);

    panes.press_in_the_app::<InstallAgentExtensions>(&mut cx);
    answer(&mut panes, Request::ListConsents, Vec::new(), &mut cx);
    wait_until(&mut cx, "the notice", |cx| {
        panes.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert_eq!(
        panes.pane.read_with(&cx, |pane, _| pane.notice()),
        Some(NOTHING_TO_INSTALL.to_owned())
    );
    // The same for the other key; the barrier shows nothing came between.
    panes.press_in_the_app::<KeepAgentExtensionsOut>(&mut cx);
    answer(&mut panes, Request::ListConsents, Vec::new(), &mut cx);
    nothing_else_was_sent(&mut panes, &mut cx);
}

#[gpui_kit::test]
fn an_answer_the_keeper_refuses_is_said_in_the_notice_and_the_question_stays(
    cx: &mut TestAppContext,
) {
    let (panes, mut cx) = open_panes(cx);
    tell(
        &panes,
        vec![consent("pi", ConsentAnswer::NotAsked)],
        &mut cx,
    );
    click_in("terminal-consent-yes", &mut cx);
    let (id, request) = panes.keeper.request(&mut cx);
    assert_eq!(request, answering("pi", ConsentAnswer::Yes));
    panes.keeper.reply(
        id,
        Err(Refusal::Failed {
            reason: "the settings are locked".to_owned(),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        panes.pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    let notice = panes.pane.read_with(&cx, |pane, _| pane.notice()).unwrap();
    assert!(notice.contains("the settings are locked"), "{notice}");
    assert!(drawn("terminal-consent", &mut cx).is_some());
}
