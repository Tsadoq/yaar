mod common;

use std::fs;
use std::path::PathBuf;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};
use yaar_link::{
    Fact, FlowForm, RequestKind, SessionId, Setting, Step, TerminalId, TurnId, WorkspaceId,
};
use yaar_review_pane::{ChangeShown, Following, PaneRow, TimelineEntry, View};
use yaar_system_tests::standin::{PROMPT, Script};
use yaar_system_tests::{Client, DEADLINE, World};

use common::{
    Composed, give_an_identity, key_of, open_another_composed_in, open_composed_in, wait_until,
};

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

fn screen_shows(
    window: &Composed,
    terminal: &TerminalId,
    mark: &str,
    cx: &mut VisualTestContext,
) -> bool {
    screen_of(window, terminal, cx).is_some_and(|lines| lines.iter().any(|l| l.contains(mark)))
}

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

struct Ended {
    session: SessionId,
    turn: TurnId,
    state: String,
    since: Option<String>,
}

fn next_turn_end(client: &mut Client) -> Ended {
    match client.wait_for_fact(DEADLINE, |fact| matches!(fact, Fact::TurnEnded { .. })) {
        Fact::TurnEnded {
            session,
            turn,
            state: Some(state),
            since,
            ..
        } => Ended {
            session,
            turn,
            state,
            since,
        },
        other => panic!("a turn ended with no state recorded: {other:?}"),
    }
}

#[gpui_kit::test]
fn a_turn_of_the_agent_shows_in_flow_step_by_step_and_as_a_live_diff(cx: &mut TestAppContext) {
    // The third turn waits at a file of the world until the test makes it; the first two end at
    // once, so the third starts from a state the second recorded.
    let mut world = World::new();
    world.agents_on_path();
    let hold: PathBuf = world.home().with_file_name("hold");
    let script = Script::parse(&format!(
        "turn\nwrite first.txt one\nend\nturn\nwrite second.txt two\nend\nturn\n\
         read notes.txt\nedit notes.txt more\nwrite new.txt hello\n\
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
    // A shell's prompt is not the answer: the test waits for the stand-in's own.
    wait_until(&mut cx, "the stand-in's prompt", |cx| {
        screen_shows(&window, &terminal, PROMPT.trim_end(), cx)
    });

    let workspace: WorkspaceId = window
        .models
        .workspaces
        .read_with(&cx, |list, _| list.selected().cloned())
        .expect("a workspace is on screen");
    let mut client = window.world.client();

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

    // Marking the first turn reviewed makes what changed since the last review count from a state
    // of its own.
    let first = next_turn_end(&mut client);
    assert_eq!(
        first.since, None,
        "no review was marked before the first turn"
    );
    let review = |client: &mut Client| {
        client
            .reviews(&workspace)
            .reviews
            .into_iter()
            .find(|review| review.agent.as_ref() == Some(&first.session))
            .expect("the session has its review")
    };
    let marked = review(&mut client).id;
    client
        .mark_reviewed(&marked)
        .expect("the review is marked reviewed");
    let reviewed = review(&mut client)
        .reviewed
        .expect("the mark names the state it reviewed");
    client.type_line(&terminal, "and the next thing");
    let second = next_turn_end(&mut client);
    assert_eq!(second.since.as_deref(), Some(reviewed.as_str()));
    assert_ne!(second.state, reviewed, "the second turn changed a file");
    client.type_line(&terminal, "do the work again");

    wait_until(&mut cx, "the four steps in the timeline", |cx| {
        window.review.read_with(cx, |pane, _| {
            let timeline = pane.timeline();
            let heading = timeline
                .iter()
                .rposition(|entry| matches!(entry, TimelineEntry::Turn { .. }));
            heading.is_some_and(|heading| timeline.len() - heading == 5)
        })
    });
    let timeline = window.review.read_with(&cx, |pane, _| {
        let timeline = pane.timeline();
        let heading = timeline
            .iter()
            .rposition(|entry| matches!(entry, TimelineEntry::Turn { .. }))
            .expect("the timeline holds a turn");
        timeline[heading..].to_vec()
    });
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
    let added = 3..=3;
    assert_eq!(lines.as_deref(), Some(std::slice::from_ref(&added)));
    assert_eq!(write, &PathBuf::from("new.txt"));
    assert_eq!(command, "echo x > shell.txt");
    let turn: TurnId = turn.clone();

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

    // A click on the bar puts the focus in the pane without pausing follow.
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
    let kept = client
        .turn(&turn)
        .expect("the Keeper holds the running turn");
    assert_eq!(
        kept.start.as_deref(),
        Some(second.state.as_str()),
        "the turn starts from the state the turn before recorded, the newest of the world"
    );
    let start = second.state.clone();
    assert_eq!(kept.state, None, "the running turn has no state yet");
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

    fs::write(&hold, "").expect("let the turn go on");
    let third = next_turn_end(&mut client);
    assert_eq!(third.turn, turn);
    assert_eq!(third.since.as_deref(), Some(reviewed.as_str()));
    let state = third.state;
    let offer = ChangeShown::Since {
        from: Some(reviewed.clone()),
        to: state.clone(),
    };
    wait_until(&mut cx, "the offer at the turn's end", |cx| {
        window
            .review
            .read_with(cx, |pane, _| pane.flow_offer() == Some(&offer))
    });
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
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Flow,
        "Flow stays when the turn ends"
    );

    cx.simulate_keystrokes("shift-o");
    wait_until(&mut cx, "Changes with the change of the turn", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.view() == View::Changes && pane.change_shown() == offer
        })
    });
    let files = ["new.txt", "notes.txt", "second.txt", "shell.txt"].map(PathBuf::from);
    wait_until(&mut cx, "the files of the change in Changes", |cx| {
        window.review.read_with(cx, |pane, _| {
            files.iter().all(|file| {
                pane.rows()
                    .iter()
                    .any(|row| matches!(row, PaneRow::File { path, .. } if path == file))
            })
        })
    });
    let mut shown: Vec<PathBuf> = window.review.read_with(&cx, |pane, _| {
        pane.rows()
            .iter()
            .filter_map(|row| match row {
                PaneRow::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    });
    shown.sort();
    assert_eq!(shown, files);
}

fn wait_for_form(client: &mut Client, workspace: &WorkspaceId, form: FlowForm) {
    let wanted = Fact::SettingChanged(Setting::FlowForm {
        workspace: workspace.clone(),
        form,
    });
    let told = client.wait_for_fact(DEADLINE, |fact| fact == &wanted);
    assert_eq!(told, wanted);
}

fn on_screen(window: &Composed, cx: &mut VisualTestContext) -> WorkspaceId {
    window
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
        .expect("a workspace is on screen")
}

#[gpui_kit::test]
fn the_form_of_flow_chosen_last_is_the_one_a_window_opens_after_the_keeper_restarts(
    app: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let mut client = world.client();
    let (mut window, mut cx) = open_composed_in(world, app, Vec::new());

    window.open_a_folder(&mut cx);
    let a = on_screen(&window, &mut cx);
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes("f");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Flow
    );
    cx.simulate_keystrokes("shift-l");
    wait_for_form(&mut client, &a, FlowForm::LiveDiff);
    wait_until(&mut cx, "the live diff shown", |cx| {
        window
            .review
            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::LiveDiff)
    });
    cx.simulate_keystrokes("shift-l");
    wait_for_form(&mut client, &a, FlowForm::Timeline);
    wait_until(&mut cx, "the timeline shown", |cx| {
        window
            .review
            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::Timeline)
    });

    cx.simulate_keystrokes("f");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Changes
    );
    window.open_a_folder(&mut cx);
    let b = on_screen(&window, &mut cx);
    assert_ne!(a, b);
    window.click_into_the_review_pane(&mut cx);
    cx.simulate_keystrokes("f");
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.view()),
        View::Flow
    );
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.flow_form()),
        FlowForm::Timeline,
        "no form was chosen in B yet"
    );
    cx.simulate_keystrokes("shift-l");
    wait_for_form(&mut client, &b, FlowForm::LiveDiff);

    // The first Window is left to find its link lost.
    let mut world = window.into_world();
    drop(client);
    world.kill_keeper();
    world.start_keeper();
    let (window, mut cx) = open_another_composed_in(world, app, Vec::new());
    wait_until(&mut cx, "both workspaces listed", |cx| {
        window.models.workspaces.read_with(cx, |list, _| {
            list.workspace(&a).is_some() && list.workspace(&b).is_some()
        })
    });
    wait_until(&mut cx, "both workspaces' forms in the settings", |cx| {
        window.models.settings.read_with(cx, |list, _| {
            [&a, &b].into_iter().all(|workspace| {
                list.all().iter().any(|setting| {
                    matches!(setting, Setting::FlowForm { workspace: of, .. } if of == workspace)
                })
            })
        })
    });

    for (workspace, form) in [(&a, FlowForm::Timeline), (&b, FlowForm::LiveDiff)] {
        let row = window.point_in(&format!("sidebar-workspace-{workspace}"), &mut cx);
        cx.simulate_click(row, Modifiers::none());
        wait_until(&mut cx, "the workspace on screen", |cx| {
            on_screen(&window, cx) == *workspace
        });
        window.click_into_the_review_pane(&mut cx);
        if window.review.read_with(&cx, |pane, _| pane.view()) != View::Flow {
            cx.simulate_keystrokes("f");
        }
        assert_eq!(
            window.review.read_with(&cx, |pane, _| pane.view()),
            View::Flow
        );
        assert_eq!(
            window.review.read_with(&cx, |pane, _| pane.flow_form()),
            form,
            "the form of Flow in {workspace}"
        );
    }
    wait_until(&mut cx, "the live diff drawn", |cx| {
        cx.debug_bounds("flow-live").is_some()
    });
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
