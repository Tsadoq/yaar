//! Component tests: Flow in the composed Window on GPUI's test platform, with no window on
//! any screen and the built Keeper program beneath it.
//!
//! The Keeper is real, and so is the agent beneath it, the stand-in, started by a terminal
//! of the Window as Claude Code is. The keys are pressed as a user presses them, and what
//! the panes show is read back through their models. The tests wait for facts with a
//! deadline and never for time.

mod common;

use std::fs;
use std::path::PathBuf;

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_link::{Fact, FlowForm, RequestKind, Setting, Step, TerminalId, TurnId, WorkspaceId};
use yaar_review_pane::{ChangeShown, Following, PaneRow, TimelineEntry, View};
use yaar_system_tests::standin::{PROMPT, Script};
use yaar_system_tests::{DEADLINE, World};

use common::{
    Composed, give_an_identity, key_of, open_another_composed_in, open_composed_in, wait_until,
};

/// The lines a terminal of the Window shows, as its emulator holds them.
fn screen_of(
    window: &Composed,
    terminal: &TerminalId,
    cx: &mut VisualTestContext,
) -> Option<Vec<String>> {
    window
        .terminals
        .read_with(cx, |pane, _| pane.terminal(terminal))
        .map(|view| view.read_with(cx, |view, _| view.lines()))
}

/// Whether some line of the terminal's screen holds `mark`.
fn screen_shows(
    window: &Composed,
    terminal: &TerminalId,
    mark: &str,
    cx: &mut VisualTestContext,
) -> bool {
    screen_of(window, terminal, cx).is_some_and(|lines| lines.iter().any(|l| l.contains(mark)))
}

/// The paths of the files the live diff lists as changed, in the order of its rows, and
/// the paths it lists as read.
fn live_files(window: &Composed, cx: &mut VisualTestContext) -> (Vec<PathBuf>, Vec<PathBuf>) {
    window.review.read_with(cx, |pane, _| {
        let mut changed = Vec::new();
        let mut read = Vec::new();
        for row in pane.live_rows() {
            match row {
                PaneRow::File { path, .. } => changed.push(path.clone()),
                PaneRow::Read { path } => read.push(path.clone()),
                _ => {}
            }
        }
        (changed, read)
    })
}

// WIN-07.
#[gpui_kit::test]
fn a_turn_of_the_agent_shows_in_flow_step_by_step_and_as_a_live_diff(cx: &mut TestAppContext) {
    // The turn waits at a file of the world, outside the worktree, until the test makes it.
    let mut world = World::new();
    world.agents_on_path();
    let hold: PathBuf = world.home().with_file_name("hold");
    let script = Script::parse(&format!(
        "turn\nread notes.txt\nedit notes.txt more\nwrite new.txt hello\n\
         shell echo x > shell.txt\nhold {}\nend\nturn",
        hold.display()
    ));
    world.agent_script("default", &script);
    let repo = world.repository("app");
    give_an_identity(&repo);
    repo.write("notes.txt", "one\ntwo\n");
    repo.commit("Add the notes");
    world.start_keeper();
    let folder = repo.path().to_owned();

    let (mut window, mut cx) = open_composed_in(world, cx, Vec::new());
    window.open_this_folder(&folder, &mut cx);
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Changes,
        "the review pane opens in Changes"
    );

    // A terminal, `claude` typed in it, and the stand-in's own prompt waited for: a shell's
    // prompt is not the answer.
    cx.simulate_keystrokes(&key_of("New Terminal"));
    wait_until(&mut cx, "the terminal to take the focus", |cx| {
        window.focus_is_in("Program", cx)
    });
    let terminal = window
        .terminals
        .read_with(&cx, |pane, _| pane.terminals())
        .into_iter()
        .next()
        .expect("the pane holds the terminal");
    cx.simulate_input("claude");
    cx.simulate_keystrokes("enter");
    wait_until(&mut cx, "the stand-in's prompt", |cx| {
        screen_shows(&window, &terminal, PROMPT.trim_end(), cx)
    });

    // The message of the user: the review pane shows Flow by itself, with follow on.
    cx.simulate_input("do the work");
    cx.simulate_keystrokes("enter");
    wait_until(&mut cx, "Flow on screen", |cx| {
        window
            .review
            .read_with(cx, |pane, _| pane.view() == View::Flow)
    });
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.following()),
        Following::On
    );

    // The timeline: the turn's heading, then its four steps in the order the stand-in did
    // them, each told as it happened.
    wait_until(&mut cx, "the four steps in the timeline", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.timeline()
                .iter()
                .filter(|entry| matches!(entry, TimelineEntry::Step { .. }))
                .count()
                == 4
        })
    });
    let timeline = window
        .review
        .read_with(&cx, |pane, _| pane.timeline().to_vec());
    let [
        TimelineEntry::Turn { turn, title, .. },
        TimelineEntry::Step {
            turn: read_in,
            number: 1,
            step: Step::Read { path: read },
        },
        TimelineEntry::Step {
            turn: edit_in,
            number: 2,
            step: Step::Edit { path: edit, lines },
        },
        TimelineEntry::Step {
            turn: write_in,
            number: 3,
            step: Step::Write { path: write },
        },
        TimelineEntry::Step {
            turn: command_in,
            number: 4,
            step: Step::Command { command },
        },
    ] = &timeline[..]
    else {
        panic!("the timeline is not the turn's heading and its four steps: {timeline:?}");
    };
    assert_eq!(title, "Claude Code 1");
    assert!(
        [read_in, edit_in, write_in, command_in]
            .into_iter()
            .all(|step_turn| step_turn == turn)
    );
    assert_eq!(read, &PathBuf::from("notes.txt"));
    assert_eq!(edit, &PathBuf::from("notes.txt"));
    // "one", "two" and the line the edit added.
    let added = 3..=3;
    assert_eq!(lines.as_deref(), Some(std::slice::from_ref(&added)));
    assert_eq!(write, &PathBuf::from("new.txt"));
    assert_eq!(command, "echo x > shell.txt");
    let turn: TurnId = turn.clone();

    // Flow is at the newest step, and the stand-in holds the turn open.
    wait_until(&mut cx, "Flow at the newest step", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.flow_at()
                .is_some_and(|at| at.turn == turn && at.number == 4)
        })
    });
    wait_until(&mut cx, "the stand-in to hold the turn", |cx| {
        screen_shows(&window, &terminal, "● Hold", cx)
    });
    assert!(!hold.exists(), "the turn is held until the test lets it go");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.following()),
        Following::On
    );

    // The live diff of the running turn, from the state the turn started from. A click on
    // the bar puts the focus in the pane without pausing follow, and the key acts in Flow.
    window.click_into_the_review_pane(&mut cx);
    assert!(
        window.focus_is_in(yaar_review_pane::KEY_CONTEXT, &mut cx)
            && !window.focus_is_in(yaar_frame::PROGRAM_CONTEXT, &mut cx),
        "the focus is in the review pane"
    );
    cx.simulate_keystrokes("shift-l");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    let mut client = window.world.client();
    let kept = client
        .turn(&turn)
        .expect("the Keeper holds the running turn");
    let start = kept.start.clone().expect("the running turn has a start");
    assert_eq!(kept.state, None, "the running turn has no state yet");
    // The files, each with the lines of its diff: a heading alone stands for a diff still
    // being read.
    wait_until(&mut cx, "the live diff of the running turn", |cx| {
        let change = window
            .review
            .read_with(cx, |pane, _| pane.live_change().cloned());
        let diffed = window.review.read_with(cx, |pane, _| {
            ["new.txt", "notes.txt", "shell.txt"].iter().all(|file| {
                pane.live_rows().iter().any(
                    |row| matches!(row, PaneRow::Diff { path, .. } if path == &PathBuf::from(file)),
                )
            })
        });
        diffed
            && change.is_some_and(|change| {
                change.turn == turn && change.from.as_deref() == Some(start.as_str())
            })
    });
    let change = window
        .review
        .read_with(&cx, |pane, _| pane.live_change().cloned())
        .expect("the live diff names the turn");
    assert_eq!(
        change.to, None,
        "the running turn is shown up to the files on disk"
    );
    let (changed, read) = live_files(&window, &mut cx);
    let mut sorted = changed.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        ["new.txt", "notes.txt", "shell.txt"].map(PathBuf::from),
        "the turn changed these three and no other file: {changed:?}"
    );
    assert_eq!(
        read,
        Vec::<PathBuf>::new(),
        "the turn read a file it changed"
    );

    // The turn ends: Flow stays, the live diff reaches the state the Keeper recorded, and
    // the pane offers what changed since the last review.
    fs::write(&hold, "").expect("let the turn go on");
    wait_until(&mut cx, "the offer at the turn's end", |cx| {
        window
            .review
            .read_with(cx, |pane, _| pane.flow_offer().is_some())
    });
    let ended = window.review.read_with(&cx, |pane, _| {
        (
            pane.view(),
            pane.live_change().cloned(),
            pane.flow_offer().cloned(),
        )
    });
    assert_eq!(ended.0, View::Flow, "Flow stays when the turn ends");
    let kept = client.turn(&turn).expect("the Keeper holds the turn");
    let state = kept.state.clone().expect("the turn's end recorded a state");
    wait_until(&mut cx, "the live diff to reach the turn's state", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.live_change()
                .is_some_and(|change| change.to.as_deref() == Some(state.as_str()))
        })
    });
    let change = window
        .review
        .read_with(&cx, |pane, _| pane.live_change().cloned())
        .expect("the live diff names the turn");
    assert_eq!(change.from.as_deref(), Some(start.as_str()));
    // Still Flow, now that the live diff has taken the turn's state too: a view that was
    // to switch away at the turn's end has done it by then.
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Flow
    );
    let offer = ended.2.expect("the turn's end offers something");
    // No review was marked, so the change counts from the history's start: the commit
    // checked out.
    assert_eq!(
        offer,
        ChangeShown::Since {
            from: Some(repo.git(&["rev-parse", "HEAD"])),
            to: state.clone()
        },
        "the offer is what changed since the reviewer's last review, of whom there was none"
    );

    // The key of the offer shows Changes with that change.
    cx.simulate_keystrokes("shift-o");
    wait_until(&mut cx, "Changes with the change of the turn", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.view() == View::Changes && pane.change_shown() == offer
        })
    });
    wait_until(&mut cx, "the files of the change in Changes", |cx| {
        window.review.read_with(cx, |pane, _| {
            ["new.txt", "notes.txt", "shell.txt"].iter().all(|file| {
                pane.rows().iter().any(
                    |row| matches!(row, PaneRow::File { path, .. } if path == &PathBuf::from(file)),
                )
            })
        })
    });
}

// WIN-08.
#[gpui_kit::test]
fn the_form_of_flow_chosen_last_is_the_one_a_window_opens_after_the_keeper_restarts(
    app: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    // A client attached to the Keeper, which hears what the Keeper tells every Window.
    let mut client = world.client();
    let (mut window, mut cx) = open_composed_in(world, app, Vec::new());
    window.open_a_folder(&mut cx);

    // Flow, and in it the other form. The key acts in Flow only.
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes("f");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Flow
    );
    cx.simulate_keystrokes("shift-l");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::LiveDiff
    );
    let workspace: WorkspaceId = window
        .models
        .workspaces
        .read_with(&cx, |list, _| list.selected().cloned())
        .expect("a workspace is on screen");
    // The Keeper tells the form once its Store holds it.
    let told = client.wait_for_fact(DEADLINE, |fact| {
        matches!(
            fact,
            Fact::SettingChanged(Setting::FlowForm {
                form: FlowForm::LiveDiff,
                ..
            })
        )
    });
    assert_eq!(
        told,
        Fact::SettingChanged(Setting::FlowForm {
            workspace: workspace.clone(),
            form: FlowForm::LiveDiff,
        })
    );

    // The Keeper ends with no goodbye and starts again on the same folders, and a new Window
    // opens against it. The first Window is left to find its link lost.
    let mut world = window.into_world();
    drop(client);
    world.kill_keeper();
    world.start_keeper();
    let (window, mut cx) = open_another_composed_in(world, app, Vec::new());
    wait_until(&mut cx, "the workspace on screen", |cx| {
        window
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&workspace))
    });
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes("f");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Flow
    );
    wait_until(
        &mut cx,
        "Flow in the form chosen before the restart",
        |cx| {
            window
                .review
                .read_with(cx, |pane, _| pane.flow_form() == FlowForm::LiveDiff)
        },
    );
    // The form shows: `flow-live` is drawn in Flow's place, which the timeline never draws.
    wait_until(&mut cx, "the live diff drawn", |cx| {
        cx.debug_bounds("flow-live").is_some()
    });
    // The settings are asked for before any turn is.
    let requests = window.tap.requests();
    let settings = requests
        .iter()
        .position(|request| request.kind() == RequestKind::GetSettings)
        .expect("the second Window asked for the settings");
    assert!(
        requests[..settings]
            .iter()
            .all(|request| request.kind() != RequestKind::GetTurn),
        "a turn was asked for before the settings: {requests:?}"
    );
}
