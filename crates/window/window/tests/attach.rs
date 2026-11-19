//! Component test: the composed Window started against a Keeper that already holds state,
//! as after a plain quit and a new start, on GPUI's test platform with no window on any
//! screen.
//!
//! The Keeper is the built program, with the stand-in agent as the agent CLI beneath it.
//! A scripted client makes what the Keeper holds, the way a Window would, and goes away
//! with no goodbye; the Window then opens against it, and what it shows is read back
//! through the models of its panes. The test waits for facts with a deadline and never for
//! time.

mod common;

use std::fs;
use std::path::PathBuf;

use gpui_kit::{TestAppContext, VisualTestContext};
use yaar_frame::SidebarRow;
use yaar_link::{
    Anchor, CommentId, DraftOn, HistoryEntry, Lines, SessionState, Side, TerminalId, TerminalSize,
    ThreadId, WorkspaceId,
};
use yaar_system_tests::standin::{PROMPT, Script};
use yaar_system_tests::{DEADLINE, World, wait_until as wait_for};

use common::{Composed, give_an_identity, open_composed_in, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// What the first terminal's shell printed, and the Window shows once it attaches.
const FIRST_SCREEN: &str = "screen-one";

/// The text of the draft the client saved in the review.
const DRAFT: &str = "keep this thought";

/// What the Keeper holds when the Window starts, as the client left it.
struct Held {
    /// The workspace.
    workspace: WorkspaceId,
    /// The terminal of the shell that printed [`FIRST_SCREEN`].
    shell: TerminalId,
    /// The terminal in which the stand-in ran a turn that ended.
    agent: TerminalId,
    /// The draft the client saved.
    comment: CommentId,
}

/// The lines each of the two terminals of the pane shows, as its emulator holds them.
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

/// Makes a world whose Keeper holds two terminals of one workspace, a draft in the review
/// of that workspace and a turn that ended while no client was attached, and leaves it with
/// no client. Returns the world, started, and what the Keeper holds.
fn a_keeper_that_holds_state() -> (World, Held) {
    let mut world = World::new();
    world.agents_on_path();
    // The turn waits at the file `go` of the world, outside the worktree, until the test
    // makes it; the stand-in then reads on for the next message.
    let go: PathBuf = world.home().with_file_name("go");
    let script = Script::parse(&format!("turn\nhold {}\nend\nturn", go.display()));
    world.agent_script("default", &script);
    let repo = world.repository("app");
    give_an_identity(&repo);
    // A change on disk for the draft to sit on.
    repo.write("notes.txt", "one\ntwo\nthree\n");
    world.start_keeper();

    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");

    let shell = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&shell, &format!("printf 'screen-%s\\n' {}", "one"));
    client.wait_for_output(&shell, DEADLINE, |bytes| {
        String::from_utf8_lossy(bytes).contains(FIRST_SCREEN)
    });

    let agent = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&agent, "claude");
    client.wait_for_prompt(&agent, 0);

    // A draft in the one review of the workspace, on a line of the file on disk.
    let reviews = client.reviews(&workspace);
    let [review] = &reviews.reviews[..] else {
        panic!("not exactly one review: {:?}", reviews.reviews);
    };
    let comment = CommentId::new(format!("{:032x}", 2));
    client
        .save_draft(
            &review.id,
            &ThreadId::new(format!("{:032x}", 1)),
            &comment,
            DraftOn::NewThread {
                anchor: Anchor {
                    tree: None,
                    path: PathBuf::from("notes.txt"),
                    side: Side::New,
                    lines: Lines { first: 2, last: 2 },
                    text: "two".to_owned(),
                },
            },
            DRAFT,
        )
        .expect("save the draft");

    // The turn starts, and the client goes away with no goodbye while the agent works.
    client.type_line(&agent, "do the work");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    client.vanish();

    // The turn ends meanwhile. A fresh client, which holds nothing of what came before,
    // is the barrier: it waits until the Keeper says the session is unreviewed and keeps
    // the turn with its state.
    fs::write(&go, "").expect("let the turn go on");
    let mut probe = world.client();
    assert!(
        wait_for(DEADLINE, || {
            let unreviewed = probe.agents().iter().any(|agents| {
                agents.workspace == workspace && agents.shown == Some(SessionState::Unreviewed)
            });
            unreviewed
                && probe
                    .history(&workspace)
                    .expect("the history")
                    .entries
                    .iter()
                    .any(|entry| matches!(entry, HistoryEntry::Turn { state: Some(_), .. }))
        }),
        "the turn did not end in the Keeper"
    );
    probe.vanish();
    (
        world,
        Held {
            workspace,
            shell,
            agent,
            comment,
        },
    )
}

/// Starts the Window against `world` and waits until the Keeper has answered it.
fn start_the_window(world: World, cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    open_composed_in(world, cx, Vec::new())
}

// WIN-06.
#[gpui_kit::test]
fn a_window_started_against_a_keeper_that_holds_two_terminals_a_draft_and_a_turn_that_ended_shows_their_screens_the_draft_and_the_finished_state(
    cx: &mut TestAppContext,
) {
    let (world, held) = a_keeper_that_holds_state();
    let (window, mut cx) = start_the_window(world, cx);

    // The terminals' screens: the pane lists both terminals the Keeper holds, shows one
    // of them, those of the workspace on screen, and each view draws what its program
    // wrote before this Window existed.
    wait_until(&mut cx, "the workspace on screen", |cx| {
        window
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&held.workspace))
    });
    wait_until(&mut cx, "both terminals of the Keeper in the pane", |cx| {
        window.terminals.read_with(cx, |pane, _| pane.terminals())
            == [held.shell.clone(), held.agent.clone()]
    });
    for terminal in [&held.shell, &held.agent] {
        assert_eq!(
            window
                .terminals
                .read_with(&cx, |pane, _| pane.workspace(terminal)),
            Some(held.workspace.clone())
        );
    }
    let shown = window.terminals.read_with(&cx, |pane, _| pane.shown());
    assert!(
        matches!(&shown[..], [one] if *one == held.shell || *one == held.agent),
        "the pane shows one of the two terminals: {shown:?}"
    );
    wait_until(&mut cx, "the screen of the shell", |cx| {
        screen_of(&window, &held.shell, cx)
            .is_some_and(|lines| lines.iter().any(|line| line.trim() == FIRST_SCREEN))
    });
    // The agent's screen ends in the prompt it printed after the turn: its whole history
    // was kept, the turn's lines before it included.
    wait_until(&mut cx, "the screen of the agent", |cx| {
        screen_of(&window, &held.agent, cx).is_some_and(|lines| {
            let marks = ["● Done", PROMPT.trim_end()];
            marks
                .iter()
                .all(|mark| lines.iter().any(|line| line.contains(mark)))
        })
    });

    // The draft: the review pane holds the review with the comment, still a draft.
    wait_until(&mut cx, "the draft in the review pane", |cx| {
        window.review.read_with(cx, |pane, _| {
            pane.review().is_some_and(|review| {
                review
                    .threads
                    .iter()
                    .flat_map(|thread| &thread.comments)
                    .any(|comment| {
                        comment.id == held.comment
                            && comment.text == DRAFT
                            && comment.round.is_none()
                    })
            })
        })
    });

    // The finished state: the sidebar row of the workspace shows the turn finished and
    // not reviewed.
    wait_until(&mut cx, "the finished state in the sidebar", |cx| {
        let sidebar = window
            .frame
            .read_with(cx, |frame, _| frame.sidebar().cloned())
            .expect("the Frame has a sidebar");
        sidebar.read_with(cx, |sidebar, cx| {
            sidebar.rows(cx).iter().any(|row| {
                matches!(row, SidebarRow::Workspace { id, agent: Some(SessionState::Unreviewed), .. }
                    if *id == held.workspace)
            })
        })
    });
}
