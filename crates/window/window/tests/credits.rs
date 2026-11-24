//! Component test: two agents that changed one file in one worktree, in the composed Window.
//! The stand-ins run as Claude Code and as pi in two shells of the Window.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_link::{Fact, SessionId, TerminalId};
use yaar_review_pane::{ChangeShown, PaneRow};
use yaar_system_tests::standin::{Did, PROMPT, Script, Step};
use yaar_system_tests::{Client, DEADLINE, World, wait_until as wait_for};

use common::{Composed, drawn, give_an_identity, key_of, wait_until};

const A_LINE: &str = "from A";
const B_LINE: &str = "from B";

fn files_listed(window: &Composed, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    window.review.read_with(cx, |pane, _| {
        pane.rows()
            .iter()
            .filter_map(|row| match row {
                PaneRow::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    })
}

fn row_of(window: &Composed, path: &str, cx: &mut VisualTestContext) -> usize {
    window.review.read_with(cx, |pane, _| {
        pane.rows()
            .iter()
            .position(|row| matches!(row, PaneRow::File { path: held, .. } if held == path))
            .unwrap_or_else(|| panic!("{path} has no heading among the rows"))
    })
}

fn marked_as_shared(window: &Composed, path: &str, cx: &mut VisualTestContext) -> bool {
    let by_the_model = window
        .review
        .read_with(cx, |pane, _| pane.changed_by_another_too(Path::new(path)));
    let row = row_of(window, path, cx);
    let drawn_on_it = drawn(&format!("review-shared-{row}"), cx).is_some();
    assert_eq!(
        by_the_model, drawn_on_it,
        "{path}: the model says {by_the_model}, the heading draws {drawn_on_it}"
    );
    by_the_model
}

fn marked_as_no_agent(window: &Composed, path: &str, cx: &mut VisualTestContext) -> bool {
    let by_the_model = window
        .review
        .read_with(cx, |pane, _| pane.made_by_no_agent(Path::new(path)));
    let row = row_of(window, path, cx);
    let drawn_on_it = drawn(&format!("review-no-agent-{row}"), cx).is_some();
    assert_eq!(
        by_the_model, drawn_on_it,
        "{path}: the model says {by_the_model}, the heading draws {drawn_on_it}"
    );
    by_the_model
}

fn shows_line(window: &Composed, path: &str, text: &str, cx: &mut VisualTestContext) -> bool {
    let rows = window.review.read_with(cx, |pane, _| pane.rows().to_vec());
    rows.iter()
        .enumerate()
        .filter(|(_, row)| matches!(row, PaneRow::Diff { path: held, .. } if held == path))
        .any(|(index, _)| drawn(&format!("review-text-{index}-new:{text}"), cx).is_some())
}

/// The rows are made after the Keeper's answers to the reads of the files, so they may still
/// be an earlier change's while the model names this one: a file that should not be there
/// fails the wait.
fn wait_for_the_files(
    window: &Composed,
    review_of: &SessionId,
    change: &ChangeShown,
    expected: &[&str],
    cx: &mut VisualTestContext,
) {
    let expected = sorted(expected);
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        let reached = window.review.read_with(cx, |pane, _| {
            pane.review_session() == Some(review_of)
                && pane.change_shown() == *change
                && pane.changed_by_another_too(Path::new("s.txt"))
                && !pane
                    .rows()
                    .iter()
                    .any(|row| matches!(row, PaneRow::Unshown { .. }))
        });
        let mut files = files_listed(window, cx);
        files.sort();
        if reached && files == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waited in vain for {change:?} of {review_of} to list {expected:?}: the pane \
             lists {files:?}, and reached the change: {reached}"
        );
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

/// The key of New Terminal is pressed with the focus in the review pane: off macOS it does
/// nothing in a terminal.
fn open_a_shell(window: &mut Composed, cx: &mut VisualTestContext) -> TerminalId {
    let before = window.terminals.read_with(cx, |pane, _| pane.terminals());
    window.click_into_the_review_pane(cx);
    cx.simulate_keystrokes(&key_of("New Terminal"));
    let mut found = None;
    wait_until(cx, "the new terminal to take the focus", |cx| {
        found = window
            .terminals
            .read_with(cx, |pane, _| pane.focused())
            .filter(|focused| !before.contains(focused));
        found.is_some() && window.focus_is_in("Program", cx)
    });
    found.expect("a terminal took the focus")
}

/// A shell's prompt is not the answer: waits for the stand-in's own.
fn start_in(
    window: &Composed,
    terminal: &TerminalId,
    name: &str,
    program: &str,
    script: &Script,
    cx: &mut VisualTestContext,
) {
    let script_file = window.world.agent_script(name, script);
    let log_file = script_file.with_extension("log");
    cx.simulate_input(&format!(
        "STANDIN_SCRIPT='{}' STANDIN_LOG='{}' {program}",
        script_file.display(),
        log_file.display()
    ));
    cx.simulate_keystrokes("enter");
    wait_until(cx, "the stand-in's prompt", |cx| {
        window.shows_on_screen(terminal, PROMPT.trim_end(), cx)
    });
}

fn session_in(
    window: &Composed,
    terminal: &TerminalId,
    cx: &mut VisualTestContext,
) -> yaar_link::Session {
    let mut found = None;
    wait_until(cx, "the session of the terminal, with its review", |cx| {
        found = window
            .models
            .agents
            .read_with(cx, |agents, _| agents.in_terminal(terminal).cloned())
            .filter(|session| session.review.is_some());
        found.is_some()
    });
    found.expect("the agents' list holds the session")
}

fn turn_end_of(client: &mut Client, session: &SessionId) -> String {
    match client.wait_for_fact(
        DEADLINE,
        |fact| matches!(fact, Fact::TurnEnded { session: ended, .. } if ended == session),
    ) {
        Fact::TurnEnded {
            state: Some(state), ..
        } => state,
        other => panic!("a turn ended with no state recorded: {other:?}"),
    }
}

fn wait_for_exit(world: &World, name: &str) {
    assert!(
        wait_for(DEADLINE, || world
            .agent_log(name)
            .did
            .iter()
            .any(|did| matches!(did, Did::Exited { .. }))),
        "{name} did not exit: {:?}",
        world.agent_log(name)
    );
}

fn sorted(paths: &[&str]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    paths.sort();
    paths
}

#[gpui_kit::test]
fn with_two_sessions_in_one_worktree_that_changed_the_same_file_each_review_shows_it_whole_with_the_mark_and_a_turn_end_selects_only_the_paths_credited_to_its_session_and_those_no_agent_made(
    cx: &mut TestAppContext,
) {
    // A's turn is held open at a gate outside the worktree while B starts, changes files and
    // ends its turn.
    let mut world = World::new();
    world.agents_on_path();
    world.install_pi();
    let gate = world.home().with_file_name("a-gate");
    let repo = world.repository("app");
    give_an_identity(&repo);
    repo.write("a.txt", "a\n");
    repo.write("s.txt", "top\n");
    repo.write("b.txt", "b\n");
    repo.commit("Add the files");
    repo.write("h.txt", "h\n");
    world.start_keeper();
    // The user's yes: pi loads the extension.
    world.allow_pi_extension();
    let folder = repo.path().to_owned();

    let (mut window, mut cx) = common::open_composed_in(world, cx, Vec::new());
    window.open_this_folder(&folder, &mut cx);
    let a_script = Script(vec![
        Step::Turn,
        Step::Edit(PathBuf::from("a.txt"), "edited by A".to_owned()),
        Step::Edit(PathBuf::from("s.txt"), A_LINE.to_owned()),
        Step::Hold(gate.clone()),
        Step::End,
        Step::Exit,
    ]);
    let b_script = Script(vec![
        Step::Turn,
        Step::Edit(PathBuf::from("b.txt"), "edited by B".to_owned()),
        Step::Edit(PathBuf::from("s.txt"), B_LINE.to_owned()),
        Step::End,
        Step::Exit,
    ]);
    let mut client = window.world.client();

    let shell_a = open_a_shell(&mut window, &mut cx);
    start_in(&window, &shell_a, "a", "claude", &a_script, &mut cx);
    let session_a = session_in(&window, &shell_a, &mut cx);
    cx.simulate_input("do the work of A");
    cx.simulate_keystrokes("enter");
    wait_until(&mut cx, "A's turn held open after its two edits", |cx| {
        window.shows_on_screen(&shell_a, "● Hold", cx)
    });
    assert!(!gate.exists(), "A's turn is held until the test lets it go");

    let shell_b = open_a_shell(&mut window, &mut cx);
    start_in(&window, &shell_b, "b", "pi", &b_script, &mut cx);
    let session_b = session_in(&window, &shell_b, &mut cx);
    assert_ne!(session_a.id, session_b.id);
    assert_ne!(session_a.agent, session_b.agent, "one of each CLI");
    cx.simulate_input("do the work of B");
    cx.simulate_keystrokes("enter");

    // B's review lists B's files and the one no agent made, not `a.txt`, though it is changed in
    // the worktree.
    let b_state = turn_end_of(&mut client, &session_b.id);
    let b_change = |window: &Composed, cx: &mut VisualTestContext| {
        window.review.read_with(cx, |pane, _| pane.change_shown())
    };
    wait_until(
        &mut cx,
        "the pane on B's change",
        |cx| matches!(b_change(&window, cx), ChangeShown::Since { to, .. } if to == b_state),
    );
    let b_change = b_change(&window, &mut cx);
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane
            .review()
            .map(|review| review.agent.clone())),
        Some(Some(session_b.id.clone())),
        "the pane shows B's review"
    );
    assert_eq!(
        repo.git(&["status", "--porcelain"])
            .lines()
            .filter(|line| line.ends_with("a.txt"))
            .count(),
        1,
        "a.txt is changed in the worktree while B's turn ends"
    );
    wait_for_the_files(
        &window,
        &session_b.id,
        &b_change,
        &["b.txt", "h.txt", "s.txt"],
        &mut cx,
    );
    window.show_the_changes_view(&mut cx);
    assert!(marked_as_shared(&window, "s.txt", &mut cx));
    assert!(!marked_as_shared(&window, "b.txt", &mut cx));
    assert!(!marked_as_shared(&window, "h.txt", &mut cx));
    wait_until(&mut cx, "A's line and B's among the rows of s.txt", |cx| {
        shows_line(&window, "s.txt", A_LINE, cx) && shows_line(&window, "s.txt", B_LINE, cx)
    });

    std::fs::write(&gate, "").expect("open the gate");
    let a_state = turn_end_of(&mut client, &session_a.id);
    let a_change = ChangeShown::Since {
        from: match &b_change {
            ChangeShown::Since { from, .. } => from.clone(),
            other => panic!("B's change is a change since a review: {other:?}"),
        },
        to: a_state,
    };
    wait_for_the_files(
        &window,
        &session_a.id,
        &a_change,
        &["a.txt", "h.txt", "s.txt"],
        &mut cx,
    );
    assert_eq!(
        repo.git(&["status", "--porcelain"])
            .lines()
            .filter(|line| line.ends_with("b.txt"))
            .count(),
        1,
        "b.txt is changed in the worktree while A's turn ends"
    );
    window.show_the_changes_view(&mut cx);
    assert!(marked_as_shared(&window, "s.txt", &mut cx));
    assert!(!marked_as_shared(&window, "a.txt", &mut cx));
    assert!(!marked_as_shared(&window, "h.txt", &mut cx));
    wait_until(&mut cx, "A's line and B's among the rows of s.txt", |cx| {
        shows_line(&window, "s.txt", A_LINE, cx) && shows_line(&window, "s.txt", B_LINE, cx)
    });

    window.show_the_review_of(&session_b.id, &mut cx);
    window.click_into(&mut cx, "review-history-uncommitted");
    wait_for_the_files(
        &window,
        &session_b.id,
        &ChangeShown::Uncommitted,
        &["b.txt", "h.txt", "s.txt"],
        &mut cx,
    );
    assert!(marked_as_shared(&window, "s.txt", &mut cx));
    assert!(!marked_as_shared(&window, "h.txt", &mut cx));
    assert!(!marked_as_shared(&window, "b.txt", &mut cx));
    assert!(marked_as_no_agent(&window, "h.txt", &mut cx));
    assert!(!marked_as_no_agent(&window, "s.txt", &mut cx));
    assert!(!marked_as_no_agent(&window, "b.txt", &mut cx));

    wait_for_exit(&window.world, "a");
    wait_for_exit(&window.world, "b");
}
