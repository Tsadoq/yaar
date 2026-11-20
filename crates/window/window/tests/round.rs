//! Component test: a review round at the agent's terminal in the composed Window on GPUI's
//! test platform, with no window on any screen.
//!
//! The Keeper is the built program, and the agent beneath it is the stand-in, started by a
//! launcher of the Window. The stand-in writes down every byte it reads from its terminal,
//! and that file is where the test reads what the program received: never the screen.
//! The test waits for facts with a deadline and never for time.

mod common;

use std::fs;
use std::path::PathBuf;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_link::{Attachment, SessionState};
use yaar_system_tests::standin::{Did, Script, StandinLog};
use yaar_system_tests::{DEADLINE, World, wait_until as wait_for};
use yaar_terminal_pane::ADDRESS_EACH_COMMENT;

use common::{Composed, drawn, key_of, wait_until};

/// The draft the test writes in the review pane.
const DRAFT: &str = "please look again";

/// The key of the control of the Window whose keystroke is `keystroke`, as the table binds
/// it on this system. Two controls share the title Detach Review, one at each pane.
fn key_with(keystroke: &str) -> String {
    yaar_window::controls()
        .into_iter()
        .find(|control| control.keystroke == keystroke)
        .unwrap_or_else(|| panic!("no control has the key {keystroke:?}"))
        .key()
}

/// What the stand-in has written down so far.
fn log_of(window: &Composed) -> StandinLog {
    window.world.agent_log("default")
}

/// Waits for a fact about what the stand-in has written down, looking again and again up to
/// the deadline: the file only gains lines.
fn wait_for_the_log(window: &Composed, what: &str, mut holds: impl FnMut(&StandinLog) -> bool) {
    assert!(
        wait_for(DEADLINE, || holds(&log_of(window))),
        "waited in vain for the stand-in's log to show {what}: {:?}",
        log_of(window)
    );
}

impl Composed {
    /// What is attached to the review the pane shows, and what the agent's terminal reads:
    /// the review pane's and the agents' list's word for it; none while they differ.
    fn attached_everywhere(&self, cx: &mut VisualTestContext) -> Option<Option<Attachment>> {
        let terminal = self.agent.as_ref().expect("an agent runs").terminal.clone();
        let in_the_pane = self
            .review
            .read_with(cx, |pane, _| pane.review().and_then(|r| r.attached.clone()));
        let at_the_terminal = self
            .terminals
            .read_with(cx, |pane, cx| pane.attachment(&terminal, cx));
        (in_the_pane == at_the_terminal).then_some(in_the_pane)
    }

    /// Waits until the review of the agent is attached as `attached` says, in both places.
    fn wait_for_attached(&self, attached: Option<Attachment>, cx: &mut VisualTestContext) {
        wait_until(cx, "the attachment the step made", |cx| {
            self.attached_everywhere(cx) == Some(attached.clone())
        });
    }

    /// Clicks into the slot of the first terminal, so that what is typed next reaches its
    /// program, and waits until the focus is there.
    fn click_into_the_terminal(&self, cx: &mut VisualTestContext) {
        let at = self.point_in("terminal-slot-0", cx);
        cx.simulate_click(at, Modifiers::none());
        wait_until(cx, "the focus in the terminal", |cx| {
            self.focus_is_in("Program", cx)
        });
    }

    /// The workspace on screen.
    fn selected(&self, cx: &mut VisualTestContext) -> Option<yaar_link::WorkspaceId> {
        self.models
            .workspaces
            .read_with(cx, |list, _| list.selected().cloned())
    }
}

// WIN-05.
#[gpui_kit::test]
fn the_program_in_a_terminal_receives_the_keys_typed_and_the_one_sentence_and_no_line_end_over_an_attach_a_view_a_detach_a_turn_end_a_notice_and_a_permission_prompt(
    cx: &mut TestAppContext,
) {
    // The stand-in holds at the first gate, outside the worktrees, and starts a turn by
    // itself when it opens: no key ends a line. The second turn asks for permission.
    let mut world = World::new();
    world.agents_on_path();
    let (first_gate, second_gate): (PathBuf, PathBuf) = (
        world.home().with_file_name("first-gate"),
        world.home().with_file_name("second-gate"),
    );
    let script = Script::parse(&format!(
        "hold {}\nself-turn\nedit notes.txt more\nend\nhold {}\nself-turn\n\
         ask p.txt text\nend\nexit",
        first_gate.display(),
        second_gate.display()
    ));
    world.agent_script("default", &script);
    world.start_keeper();
    let (mut window, mut cx) = common::open_composed_in(world, cx, Vec::new());

    // Two workspaces, the first on screen, and the agent in its terminal.
    window.open_a_folder(&mut cx);
    let first = window.selected(&mut cx).expect("a workspace is on screen");
    window.open_a_folder(&mut cx);
    let second = window.selected(&mut cx).expect("a workspace is on screen");
    assert_ne!(first, second);
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes(&key_of("Previous Workspace"));
    wait_until(&mut cx, "the first workspace on screen", |cx| {
        window.selected(cx).as_ref() == Some(&first)
    });
    window.start_an_agent(&mut cx);
    let agent = window.agent.as_ref().expect("the agent runs");
    let (terminal, review) = (agent.terminal.clone(), agent.review.clone());
    assert_eq!(
        window
            .review
            .read_with(&cx, |pane, _| pane.review().map(|r| r.agent.clone())),
        Some(Some(agent.session.clone())),
        "the review pane shows the review of the agent's session"
    );

    // Keys typed with no Enter: the stand-in holds them.
    window.click_into_the_terminal(&mut cx);
    cx.simulate_input("abc");
    wait_for_the_log(&window, "the keys typed", |log| log.received == b"abc");

    // Over an attach: a draft in the review pane, and the key of Attach Review.
    window.click_into(&mut cx, "review-row-0");
    cx.simulate_keystrokes("c");
    cx.simulate_input(DRAFT);
    cx.simulate_keystrokes("enter");
    wait_until(&mut cx, "the draft in the review", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.review().is_some_and(|review| {
                review
                    .threads
                    .iter()
                    .flat_map(|thread| &thread.comments)
                    .any(|comment| comment.text == DRAFT && comment.round.is_none())
            })
        })
    });
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes("a");
    window.wait_for_attached(Some(Attachment::Review), &mut cx);
    wait_until(&mut cx, "the line at the terminal that says so", |cx| {
        drawn("terminal-attachment-0", cx).is_some()
    });

    // A view, and the same key again to close it. It shows what will be sent, which
    // asks the Keeper and types nothing.
    cx.simulate_keystrokes(&key_with("secondary-alt-v"));
    wait_until(&mut cx, "the view of the attachment", |cx| {
        window
            .terminals
            .read_with(cx, |pane, _| pane.viewed_attachment(&terminal))
            .is_some_and(|text| text.contains(DRAFT))
            && drawn("terminal-attachment-text-0", cx).is_some()
    });
    cx.simulate_keystrokes(&key_with("secondary-alt-v"));
    wait_until(&mut cx, "the view closed", |cx| {
        window
            .terminals
            .read_with(cx, |pane, _| pane.viewed_attachment(&terminal))
            .is_none()
            && drawn("terminal-attachment-text-0", cx).is_none()
    });

    // A detach, by the key at the terminal, and an attach again.
    cx.simulate_keystrokes(&key_with("secondary-alt-x"));
    window.wait_for_attached(None, &mut cx);
    wait_until(&mut cx, "the line at the terminal gone", |cx| {
        drawn("terminal-attachment-0", cx).is_none()
    });
    cx.simulate_keystrokes("a");
    window.wait_for_attached(Some(Attachment::Review), &mut cx);

    // The one-click control. The agent works at no prompt that asks, so it is offered.
    wait_until(&mut cx, "the control that types the sentence", |cx| {
        window
            .terminals
            .read_with(cx, |pane, cx| pane.offers_address(&terminal, cx))
            && drawn("terminal-attachment-address-0", cx).is_some()
    });
    let at = window.point_in("terminal-attachment-address-0", &mut cx);
    cx.simulate_click(at, Modifiers::none());
    let typed = format!("abc{ADDRESS_EACH_COMMENT}");
    wait_for_the_log(&window, "the sentence", |log| {
        log.received == typed.as_bytes()
    });

    // A turn end in a workspace that is not on screen, which no key ends: the notice.
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes(&key_of("Next Workspace"));
    wait_until(&mut cx, "the second workspace on screen", |cx| {
        window.selected(cx).as_ref() == Some(&second)
    });
    fs::write(&first_gate, "").expect("open the first gate");
    wait_until(&mut cx, "the notice of the turn that ended", |cx| {
        drawn("frame-turn-notice", cx).is_some()
    });
    let at = window.point_in("frame-turn-notice-show", &mut cx);
    cx.simulate_click(at, Modifiers::none());
    wait_until(&mut cx, "the first workspace on screen again", |cx| {
        window.selected(cx).as_ref() == Some(&first) && drawn("frame-turn-notice", cx).is_none()
    });

    // A permission prompt, from a turn the agent starts itself again: the app offers no
    // sentence while it is up, and the test answers it with its one key.
    fs::write(&second_gate, "").expect("open the second gate");
    wait_for_the_log(&window, "the permission request", |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::Hook { event, .. } if event == "PermissionRequest"))
    });
    wait_until(&mut cx, "the agent shown as waiting for permission", |cx| {
        window.models.agents.read_with(cx, |agents, _| {
            agents
                .in_terminal(&terminal)
                .is_some_and(|session| session.state == SessionState::WaitsForPermission)
        })
    });
    assert!(
        !window
            .terminals
            .read_with(&cx, |pane, cx| pane.offers_address(&terminal, cx)),
        "no sentence is offered at a permission prompt"
    );
    assert!(drawn("terminal-attachment-address-0", &mut cx).is_none());
    window.click_into_the_terminal(&mut cx);
    cx.simulate_input("y");

    // The agent ends its session after the turn, which is the barrier: everything the test
    // typed has reached it by then.
    wait_for_the_log(&window, "the end of the agent", |log| {
        log.did.iter().any(|did| matches!(did, Did::Exited { .. }))
    });
    let received = log_of(&window).received;
    let expected = format!("abc{ADDRESS_EACH_COMMENT}y");
    assert_eq!(
        String::from_utf8_lossy(&received),
        expected,
        "the program received the keys the test typed, the sentence and nothing else"
    );
    assert!(
        !received.contains(&b'\r') && !received.contains(&b'\n'),
        "no line end reached the program"
    );
    // The agent started two turns of its own after the attach, and neither carried the
    // review: the Keeper, asked by another client once the hooks of both have answered,
    // still holds the draft unsent and the review attached.
    let reviews = window.world.client().reviews(&first);
    let held = reviews
        .reviews
        .iter()
        .find(|held| held.id == review)
        .expect("the Keeper holds the review");
    assert_eq!(held.attached, Some(Attachment::Review));
    assert!(
        held.threads
            .iter()
            .flat_map(|thread| &thread.comments)
            .any(|comment| comment.text == DRAFT && comment.round.is_none()),
        "the draft is still a draft"
    );
}
