//! Component test: what the Frame draws and handles in slice 9, on GPUI's test platform:
//! the settings screen in the place of the panes, the field for a new worktree's branch,
//! and the place of the panes as a setting. The panes are written here, as in the layout
//! test; the Keeper is the one the test plays, and the screen itself is not read for a row:
//! the contract's screen draws only its selector, which is all these tests need.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    AppContext, Context, FocusHandle, InteractiveElement, IntoElement, Pixels, Render, Size,
    Styled, TestAppContext, VisualTestContext, Window, div, px,
};
use yaar_frame::{
    Beside, BuildPane, CloseSettings, FocusNextPane, FocusPreviousPane, MovePanes, NewWorktree,
    OpenFolder, OpenSettings, Pane, SetBase, Wanted,
};
use yaar_link::{
    AgentCli, Fact, Head, Hello, PROTOCOL_VERSION, Refusal, Repository, RepositoryId, Request,
    Setting, TerminalPlace, Workspace,
};

use common::{
    Shown, drawn, id, open_frame, open_frame_staged, pressed, repository, wait_until, workspace,
};

const WINDOW: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

/// The focus handle of each pane the test's builders made, in the order they were built.
type Handles = Rc<RefCell<Vec<FocusHandle>>>;

/// A pane that draws nothing but its room.
struct Plain {
    focus: FocusHandle,
}

impl Render for Plain {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

/// A pane of this name, whose focus handle is also put in `handles`.
fn plain(name: &'static str, handles: &Handles) -> BuildPane {
    let handles = handles.clone();
    Box::new(move |_, _, cx| {
        let focus = cx.focus_handle();
        handles.borrow_mut().push(focus.clone());
        let view = cx.new(|_| Plain {
            focus: focus.clone(),
        });
        Pane {
            name,
            view: view.into(),
            focus,
        }
    })
}

fn two_panes(handles: &Handles) -> Vec<BuildPane> {
    vec![plain("first", handles), plain("second", handles)]
}

fn hello() -> Hello {
    Hello {
        protocol: PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    }
}

fn is_drawn(selector: &str, cx: &mut VisualTestContext) -> bool {
    drawn(selector, cx).is_some()
}

fn has_focus(handle: &FocusHandle, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| handle.is_focused(window))
}

fn screen_is_open(shown: &Shown, cx: &mut VisualTestContext) -> bool {
    shown
        .frame
        .read_with(cx, |frame, _| frame.settings_screen().is_some())
}

fn beside(shown: &Shown, cx: &mut VisualTestContext) -> Beside {
    shown.frame.read_with(cx, |frame, _| frame.beside())
}

fn notice(shown: &Shown, cx: &mut VisualTestContext) -> Option<String> {
    shown.frame.read_with(cx, |frame, cx| frame.notice(cx))
}

/// A window with `panes` whose Keeper has answered the question for the list with
/// `listed`, and has been asked for the changes of the workspace that came on screen.
fn showing(
    listed: &[Repository],
    panes: Vec<BuildPane>,
    clis: Vec<AgentCli>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    let (mut shown, mut cx) = open_frame_staged(hello(), WINDOW, panes, cx, |keeper| {
        keeper.set_clis(clis);
    });
    shown.keeper.list(listed, &mut cx);
    wait_until(&mut cx, "the list of workspaces", |cx| {
        shown
            .models
            .workspaces
            .read_with(cx, |list, _| list.repositories() == listed)
    });
    if let Some(first) = listed.iter().flat_map(|r| &r.workspaces).next() {
        shown.keeper.expect(
            Request::GetChanges {
                workspace: first.id.clone(),
            },
            &mut cx,
        );
    }
    (shown, cx)
}

fn cli(agent: &str, title: &str) -> AgentCli {
    AgentCli {
        agent: agent.to_owned(),
        title: title.to_owned(),
    }
}

// The settings screen.

#[gpui_kit::test]
fn the_key_of_settings_draws_the_screen_in_the_place_of_the_panes_and_gives_it_the_focus(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);

    // Before: the panes are drawn, the screen is not, and the first pane has the focus.
    assert!(is_drawn("frame-pane-first", &mut cx));
    assert!(is_drawn("frame-pane-second", &mut cx));
    assert!(!is_drawn("settings-screen", &mut cx));
    assert!(!screen_is_open(&shown, &mut cx));
    assert!(has_focus(&handles.borrow()[0], &mut cx));

    cx.simulate_keystrokes(&pressed::<OpenSettings>());

    assert!(screen_is_open(&shown, &mut cx));
    let screen = drawn("settings-screen", &mut cx).expect("the screen is drawn");
    assert!(!is_drawn("frame-pane-first", &mut cx), "no pane is drawn");
    assert!(!is_drawn("frame-pane-second", &mut cx), "no pane is drawn");
    // Where the panes were: to the right of the sidebar, which is still there.
    let sidebar = drawn("sidebar", &mut cx).expect("the sidebar is drawn");
    assert!(screen.left() >= sidebar.right());
    assert_eq!(screen.right(), WINDOW.width);
    // The focus is in the screen, and in no pane.
    let screen_focus = shown.frame.read_with(&cx, |frame, cx| {
        frame
            .settings_screen()
            .expect("the screen")
            .read(cx)
            .focus()
            .clone()
    });
    assert!(has_focus(&screen_focus, &mut cx));
    assert!(!has_focus(&handles.borrow()[0], &mut cx));
    // No dialog and no second window.
    assert!(!cx.has_pending_prompt());
    assert_eq!(cx.windows().len(), 1);
}

#[gpui_kit::test]
fn the_key_of_settings_again_closes_the_screen_and_the_focus_goes_back_where_it_was(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);
    // Opened from the second pane, which is not where the window starts.
    cx.simulate_keystrokes(&pressed::<FocusNextPane>());
    assert!(has_focus(&handles.borrow()[1], &mut cx));

    cx.simulate_keystrokes(&pressed::<OpenSettings>());
    assert!(screen_is_open(&shown, &mut cx));
    assert!(!has_focus(&handles.borrow()[1], &mut cx));

    cx.simulate_keystrokes(&pressed::<OpenSettings>());

    assert!(!screen_is_open(&shown, &mut cx));
    assert!(!is_drawn("settings-screen", &mut cx));
    assert!(is_drawn("frame-pane-first", &mut cx));
    assert!(is_drawn("frame-pane-second", &mut cx));
    assert!(
        has_focus(&handles.borrow()[1], &mut cx),
        "the focus is back"
    );
    assert!(!has_focus(&handles.borrow()[0], &mut cx));
}

#[gpui_kit::test]
fn the_action_that_closes_the_settings_closes_the_screen_and_the_focus_goes_back_where_it_was(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);
    cx.simulate_keystrokes(&pressed::<FocusNextPane>());

    cx.simulate_keystrokes(&pressed::<OpenSettings>());
    assert!(screen_is_open(&shown, &mut cx));
    // The key of this action is the screen's own, bound with the screen's table: what the
    // Frame owns is the handling, which this test reaches through the focus the screen has.
    cx.dispatch_action(CloseSettings);
    cx.run_until_parked();

    assert!(!screen_is_open(&shown, &mut cx));
    assert!(!is_drawn("settings-screen", &mut cx));
    assert!(is_drawn("frame-pane-first", &mut cx));
    assert!(is_drawn("frame-pane-second", &mut cx));
    assert!(
        has_focus(&handles.borrow()[1], &mut cx),
        "the focus is back"
    );
}

#[gpui_kit::test]
fn closing_the_settings_with_none_open_does_nothing_and_each_opening_makes_a_screen_that_the_closing_drops(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);

    cx.dispatch_action(CloseSettings);
    cx.run_until_parked();
    assert!(!screen_is_open(&shown, &mut cx));
    assert!(has_focus(&handles.borrow()[0], &mut cx), "the focus stays");

    let key = pressed::<OpenSettings>();
    cx.simulate_keystrokes(&key);
    let first = shown
        .frame
        .read_with(&cx, |frame, _| frame.settings_screen().cloned())
        .expect("the screen");
    let (first_id, first_weak) = (first.entity_id(), first.downgrade());
    drop(first);
    cx.simulate_keystrokes(&key);
    assert!(!screen_is_open(&shown, &mut cx));
    assert!(
        first_weak.upgrade().is_none(),
        "the screen is dropped at the closing"
    );

    cx.simulate_keystrokes(&key);
    let second = shown
        .frame
        .read_with(&cx, |frame, _| frame.settings_screen().cloned())
        .expect("the screen");
    assert_ne!(second.entity_id(), first_id, "a new screen at each opening");
}

#[gpui_kit::test]
fn the_keys_that_move_the_focus_between_the_panes_leave_it_in_the_screen_while_it_is_open(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);
    cx.simulate_keystrokes(&pressed::<OpenSettings>());
    let screen_focus = shown.frame.read_with(&cx, |frame, cx| {
        frame
            .settings_screen()
            .expect("the screen")
            .read(cx)
            .focus()
            .clone()
    });

    // No pane is drawn, so none is given keys nobody can see.
    cx.simulate_keystrokes(&pressed::<FocusNextPane>());
    cx.simulate_keystrokes(&pressed::<FocusPreviousPane>());

    assert!(has_focus(&screen_focus, &mut cx));
    assert!(!has_focus(&handles.borrow()[0], &mut cx));
    assert!(!has_focus(&handles.borrow()[1], &mut cx));
}

#[gpui_kit::test]
fn the_field_for_the_base_opened_over_the_screen_gives_the_focus_back_to_the_screen(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = showing(&one_repository(), two_panes(&handles), Vec::new(), cx);
    cx.simulate_keystrokes(&pressed::<OpenSettings>());
    let screen_focus = shown.frame.read_with(&cx, |frame, cx| {
        frame
            .settings_screen()
            .expect("the screen")
            .read(cx)
            .focus()
            .clone()
    });

    // The field is drawn above the screen, which stays open; Escape closes the field, and
    // the focus is where it was: in the screen, not in a pane that is not drawn.
    cx.simulate_keystrokes(&pressed::<SetBase>());
    assert!(is_drawn("frame-base-field", &mut cx));
    assert!(is_drawn("settings-screen", &mut cx));
    assert!(!has_focus(&screen_focus, &mut cx));
    cx.simulate_keystrokes("escape");

    assert!(!is_drawn("frame-base-field", &mut cx));
    assert!(screen_is_open(&shown, &mut cx));
    assert!(has_focus(&screen_focus, &mut cx));
}

// The field for a new worktree's branch.

/// What the Frame wanted of the terminals so far, and the marker that `barrier` puts after.
fn wanted_log(shown: &Shown, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<Wanted>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let log = log.clone();
        cx.subscribe(&shown.models.terminals, move |_, wanted: &Wanted, _| {
            log.borrow_mut().push(wanted.clone());
        })
        .detach();
    });
    log
}

fn worktree(repository: &RepositoryId, from: &str, branch: &str, agent: &str) -> Wanted {
    Wanted::Worktree {
        repository: repository.clone(),
        from: from.to_owned(),
        branch: branch.to_owned(),
        agent: agent.to_owned(),
    }
}

/// Wants a worktree through the same list the Frame wants through, and runs the app until
/// every subscriber has heard it. Whatever the Frame wanted before has been heard by then,
/// and stands before it in the log: its arrival shows that nothing is on its way.
fn barrier(shown: &Shown, cx: &mut VisualTestContext) -> Wanted {
    let marker = worktree(
        &RepositoryId::new("z".repeat(32)),
        "barrier",
        "barrier",
        "none",
    );
    let sent = marker.clone();
    shown
        .models
        .terminals
        .update(cx, |list, cx| list.want(sent, cx));
    cx.run_until_parked();
    marker
}

fn one_repository() -> Vec<Repository> {
    vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature/login")],
    )]
}

/// Waits until the Window has the CLIs the Keeper named.
fn wait_for_clis(shown: &Shown, cx: &mut VisualTestContext, count: usize) {
    wait_until(cx, "the agent CLIs", |cx| {
        shown
            .models
            .agents
            .read_with(cx, |list, _| list.clis().len() == count)
    });
}

fn field_is_open(cx: &mut VisualTestContext) -> bool {
    is_drawn("frame-worktree-field", cx)
}

fn field_is_open_for_base(cx: &mut VisualTestContext) -> bool {
    is_drawn("frame-base-field", cx)
}

#[gpui_kit::test]
fn a_branch_typed_in_the_field_of_a_new_worktree_wants_a_worktree_from_the_branch_on_screen_with_the_chosen_cli(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let listed = one_repository();
    let repository = listed[0].id.clone();
    let (shown, mut cx) = showing(
        &listed,
        two_panes(&handles),
        vec![cli("claude", "Claude Code"), cli("pi", "pi")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 2);
    let wanted = wanted_log(&shown, &mut cx);
    let key = pressed::<NewWorktree>();

    // The first CLI, the branch of the first worktree, a name with blanks around it.
    assert!(!field_is_open(&mut cx));
    cx.simulate_keystrokes(&key);
    assert!(field_is_open(&mut cx));
    cx.simulate_input(" task/one ");
    cx.simulate_keystrokes("enter");
    let marker = barrier(&shown, &mut cx);
    assert_eq!(
        *wanted.borrow(),
        [worktree(&repository, "main", "task/one", "claude"), marker]
    );
    assert!(!field_is_open(&mut cx), "Enter closes the field");
    assert!(
        has_focus(&handles.borrow()[0], &mut cx),
        "the focus is back where it was"
    );
    assert_eq!(notice(&shown, &mut cx), None);
    wanted.borrow_mut().clear();

    // The next CLI of the list, and the other worktree, which has a branch with a slash.
    shown
        .models
        .agents
        .update(&mut cx, |list, cx| list.choose_next_cli(cx));
    shown
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    cx.simulate_keystrokes(&key);
    cx.simulate_input("task/two");
    cx.simulate_keystrokes("enter");
    let marker = barrier(&shown, &mut cx);
    assert_eq!(
        *wanted.borrow(),
        [
            worktree(&repository, "feature/login", "task/two", "pi"),
            marker
        ]
    );
}

#[gpui_kit::test]
fn a_field_given_up_or_left_empty_wants_no_worktree(cx: &mut TestAppContext) {
    let handles = Handles::default();
    let listed = one_repository();
    let repository = listed[0].id.clone();
    let (shown, mut cx) = showing(
        &listed,
        two_panes(&handles),
        vec![cli("claude", "Claude Code")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 1);
    let wanted = wanted_log(&shown, &mut cx);
    let key = pressed::<NewWorktree>();

    // Escape gives up: the field closes, what was typed is gone, the focus goes back.
    cx.simulate_keystrokes(&key);
    cx.simulate_input("givenup");
    cx.simulate_keystrokes("escape");
    assert!(!field_is_open(&mut cx), "Escape closes the field");
    assert!(has_focus(&handles.borrow()[0], &mut cx));

    // Enter in an empty field, or one that holds blanks only, wants nothing and leaves
    // the field open; the key again while it is open leaves it as it is.
    cx.simulate_keystrokes(&key);
    cx.simulate_keystrokes("enter");
    assert!(field_is_open(&mut cx));
    cx.simulate_input("  ");
    cx.simulate_keystrokes("enter");
    assert!(field_is_open(&mut cx));
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("ta");
    cx.simulate_keystrokes(&key);
    cx.simulate_input("sk");
    cx.simulate_keystrokes("enter");

    // The one thing wanted is the last name typed, in the field that stayed.
    let marker = barrier(&shown, &mut cx);
    assert_eq!(
        *wanted.borrow(),
        [worktree(&repository, "main", "task", "claude"), marker]
    );
}

#[gpui_kit::test]
fn with_a_detached_workspace_on_screen_the_notice_says_so_and_no_worktree_is_wanted(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let detached = Workspace {
        head: Some(Head::Detached("a".repeat(40))),
        ..workspace('1', "unused")
    };
    let mut listed = one_repository();
    listed[0].workspaces.insert(0, detached);
    let repository = listed[0].id.clone();
    let (shown, mut cx) = showing(
        &listed,
        two_panes(&handles),
        vec![cli("claude", "Claude Code")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 1);
    // The workspace that came on screen is the first, which has no branch.
    let wanted = wanted_log(&shown, &mut cx);
    let key = pressed::<NewWorktree>();

    // The key alone gives the reason and opens no field; what is typed after it, and
    // Enter, go nowhere.
    cx.simulate_keystrokes(&key);
    assert!(!field_is_open(&mut cx), "no field asks for a name");
    let said = notice(&shown, &mut cx).expect("the notice says why");
    assert!(
        said.contains("no branch"),
        "the notice names the reason: {said}"
    );
    cx.simulate_input("task");
    cx.simulate_keystrokes("enter");
    assert!(!field_is_open(&mut cx));
    let marker = barrier(&shown, &mut cx);
    assert_eq!(*wanted.borrow(), [marker], "nothing was wanted");

    // A workspace with a branch comes on screen: the next try is made, and takes the
    // notice away.
    wanted.borrow_mut().clear();
    shown
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    cx.simulate_keystrokes(&key);
    cx.simulate_input("task");
    cx.simulate_keystrokes("enter");
    let marker = barrier(&shown, &mut cx);
    assert_eq!(
        *wanted.borrow(),
        [
            worktree(&repository, "feature/login", "task", "claude"),
            marker
        ]
    );
    assert_eq!(notice(&shown, &mut cx), None);
}

#[gpui_kit::test]
fn a_workspace_that_comes_on_screen_with_no_branch_while_the_field_is_open_has_the_notice_say_so_at_enter(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let detached = Workspace {
        head: Some(Head::Detached("b".repeat(40))),
        ..workspace('3', "unused")
    };
    let mut listed = one_repository();
    listed[0].workspaces.push(detached);
    let (shown, mut cx) = showing(
        &listed,
        two_panes(&handles),
        vec![cli("claude", "Claude Code")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 1);
    let wanted = wanted_log(&shown, &mut cx);

    // The field opens on the branch `main`; what is on screen at Enter is what counts.
    cx.simulate_keystrokes(&pressed::<NewWorktree>());
    assert!(field_is_open(&mut cx));
    cx.simulate_input("task");
    shown
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('3'), cx));
    cx.simulate_keystrokes("enter");

    let said = notice(&shown, &mut cx).expect("the notice says why");
    assert!(said.contains("no branch"), "{said}");
    assert!(!field_is_open(&mut cx), "the field is closed");
    let marker = barrier(&shown, &mut cx);
    assert_eq!(*wanted.borrow(), [marker], "nothing was wanted");
}

#[gpui_kit::test]
fn with_no_workspace_on_screen_the_notice_says_so_and_no_field_opens(cx: &mut TestAppContext) {
    let handles = Handles::default();
    let (shown, mut cx) = showing(
        &[],
        two_panes(&handles),
        vec![cli("claude", "Claude Code")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 1);
    let wanted = wanted_log(&shown, &mut cx);

    cx.simulate_keystrokes(&pressed::<NewWorktree>());
    assert!(!field_is_open(&mut cx), "no field asks for a name");
    let said = notice(&shown, &mut cx).expect("the notice says why");
    assert!(
        said.contains("No worktree is on screen"),
        "the notice names the reason: {said}"
    );
    cx.simulate_input("task");
    cx.simulate_keystrokes("enter");
    assert!(!field_is_open(&mut cx));
    let marker = barrier(&shown, &mut cx);
    assert_eq!(*wanted.borrow(), [marker]);
}

#[gpui_kit::test]
fn with_no_agent_cli_the_notice_says_so_and_no_field_opens(cx: &mut TestAppContext) {
    let handles = Handles::default();
    let (shown, mut cx) = showing(&one_repository(), two_panes(&handles), Vec::new(), cx);
    // The Keeper names no CLI: the question was answered with none, as it is at each
    // connect, and the list has nothing to choose from.
    shown
        .models
        .agents
        .read_with(&cx, |list, _| assert!(list.chosen_cli().is_none()));
    let wanted = wanted_log(&shown, &mut cx);

    cx.simulate_keystrokes(&pressed::<NewWorktree>());
    assert!(!field_is_open(&mut cx), "no field asks for a name");
    let said = notice(&shown, &mut cx).expect("the notice says why");
    assert!(
        said.contains("agent CLI"),
        "the notice names the reason: {said}"
    );
    cx.simulate_input("task");
    cx.simulate_keystrokes("enter");
    assert!(!field_is_open(&mut cx));
    let marker = barrier(&shown, &mut cx);
    assert_eq!(*wanted.borrow(), [marker]);
}

#[gpui_kit::test]
fn with_a_workspace_that_is_gone_on_screen_the_notice_says_so_and_no_field_opens(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let gone = Workspace {
        head: None,
        ..workspace('1', "gone")
    };
    let mut listed = one_repository();
    listed[0].workspaces.insert(0, gone);
    let (shown, mut cx) = showing(
        &listed,
        two_panes(&handles),
        vec![cli("claude", "Claude Code")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 1);
    // The workspace that came on screen is the first, which is gone.
    let wanted = wanted_log(&shown, &mut cx);

    cx.simulate_keystrokes(&pressed::<NewWorktree>());
    assert!(!field_is_open(&mut cx), "no field asks for a name");
    let said = notice(&shown, &mut cx).expect("the notice says why");
    assert!(
        said.contains("is gone"),
        "the notice names the reason: {said}"
    );
    cx.simulate_input("task");
    cx.simulate_keystrokes("enter");
    assert!(!field_is_open(&mut cx));
    let marker = barrier(&shown, &mut cx);
    assert_eq!(*wanted.borrow(), [marker], "nothing was wanted");
}

#[gpui_kit::test]
fn a_notice_of_a_new_worktree_is_taken_away_when_the_user_asks_the_keeper_for_the_next_thing(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let detached = Workspace {
        head: Some(Head::Detached("a".repeat(40))),
        ..workspace('1', "unused")
    };
    let mut listed = one_repository();
    listed[0].workspaces.insert(0, detached);
    let (mut shown, mut cx) = showing(
        &listed,
        two_panes(&handles),
        vec![cli("claude", "Claude Code")],
        cx,
    );
    wait_for_clis(&shown, &mut cx, 1);

    // A new base: the notice of the worktree is gone as soon as the user sends it, and
    // the reason the Keeper gives is the one that shows, not the older sentence.
    cx.simulate_keystrokes(&pressed::<NewWorktree>());
    assert!(notice(&shown, &mut cx).is_some());
    cx.simulate_keystrokes(&pressed::<SetBase>());
    assert!(field_is_open_for_base(&mut cx));
    cx.simulate_input("origin/none");
    cx.simulate_keystrokes("enter");
    assert_eq!(notice(&shown, &mut cx), None);
    let asked = shown.keeper.expect(
        Request::SetBase {
            workspace: id('1'),
            base: "origin/none".to_owned(),
        },
        &mut cx,
    );
    let refusal = Refusal::UnknownRevision {
        revision: "origin/none".to_owned(),
    };
    shown.keeper.reply(asked, Err(refusal.clone()));
    wait_until(&mut cx, "the Keeper's reason", |cx| {
        shown.frame.read_with(cx, |frame, cx| frame.notice(cx)) == Some(refusal.to_string())
    });

    // A folder: the same, before any answer.
    cx.simulate_keystrokes(&pressed::<NewWorktree>());
    assert!(notice(&shown, &mut cx).is_some_and(|said| said.contains("no branch")));
    cx.simulate_keystrokes(&pressed::<OpenFolder>());
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/gits/other")]));
    shown.keeper.expect(
        Request::OpenFolder {
            path: PathBuf::from("/gits/other"),
        },
        &mut cx,
    );
    assert_eq!(notice(&shown, &mut cx), None);
}

// The place of the panes.

#[gpui_kit::test]
fn the_key_of_move_panes_sets_the_next_place_through_the_settings_and_the_panes_lie_there_at_the_next_frame(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);
    let key = pressed::<MovePanes>();
    assert_eq!(beside(&shown, &mut cx), Beside::Right);

    cx.simulate_keystrokes(&key);

    // The Keeper is asked to keep it, and the model has it at once.
    wait_until(&mut cx, "the set", |_| {
        !shown.keeper.settings_set().is_empty()
    });
    assert_eq!(
        shown.keeper.settings_set(),
        [Setting::TerminalPlace(TerminalPlace::Below)]
    );
    assert_eq!(
        shown
            .models
            .settings
            .read_with(&cx, |settings, _| settings.terminal_place()),
        TerminalPlace::Below
    );
    assert_eq!(beside(&shown, &mut cx), Beside::Below);
    let (first, second) = (
        drawn("frame-pane-first", &mut cx).expect("the first pane"),
        drawn("frame-pane-second", &mut cx).expect("the second pane"),
    );
    assert!(
        second.top() >= first.bottom(),
        "below: {first:?} {second:?}"
    );
    assert_eq!(second.left(), first.left());
}

#[gpui_kit::test]
fn a_place_the_keeper_tells_moves_the_panes_and_the_key_goes_on_from_that_place(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (mut shown, mut cx) = open_frame(WINDOW, two_panes(&handles), cx);

    // Another window's choice, as a Keeper tells it: it keeps it, so a question for the
    // settings that the Window asks again is answered with it.
    shown
        .keeper
        .set_settings(vec![Setting::TerminalPlace(TerminalPlace::Left)]);
    shown
        .keeper
        .tell(Fact::SettingChanged(Setting::TerminalPlace(
            TerminalPlace::Left,
        )));
    wait_until(&mut cx, "the panes to the left", |cx| {
        cx.run_until_parked();
        shown.frame.read_with(cx, |frame, _| frame.beside()) == Beside::Left
    });
    let (first, second) = (
        drawn("frame-pane-first", &mut cx).expect("the first pane"),
        drawn("frame-pane-second", &mut cx).expect("the second pane"),
    );
    assert!(second.right() <= first.left(), "left: {first:?} {second:?}");
    assert_eq!(second.top(), first.top());

    // The key moves them on from there, not from the place this window last set.
    cx.simulate_keystrokes(&pressed::<MovePanes>());
    wait_until(&mut cx, "the set", |_| {
        !shown.keeper.settings_set().is_empty()
    });
    assert_eq!(
        shown.keeper.settings_set(),
        [Setting::TerminalPlace(TerminalPlace::Right)]
    );
    assert_eq!(beside(&shown, &mut cx), Beside::Right);
    let (first, second) = (
        drawn("frame-pane-first", &mut cx).expect("the first pane"),
        drawn("frame-pane-second", &mut cx).expect("the second pane"),
    );
    assert!(
        second.left() >= first.right(),
        "right: {first:?} {second:?}"
    );
}

#[gpui_kit::test]
fn a_place_the_keeper_holds_is_where_the_panes_lie_from_the_first_frame_after_it_answers(
    cx: &mut TestAppContext,
) {
    let handles = Handles::default();
    let (shown, mut cx) = open_frame_staged(hello(), WINDOW, two_panes(&handles), cx, |keeper| {
        keeper.set_settings(vec![Setting::TerminalPlace(TerminalPlace::Below)]);
    });

    wait_until(&mut cx, "the panes below", |cx| {
        shown.frame.read_with(cx, |frame, _| frame.beside()) == Beside::Below
    });
    let (first, second) = (
        drawn("frame-pane-first", &mut cx).expect("the first pane"),
        drawn("frame-pane-second", &mut cx).expect("the second pane"),
    );
    assert!(
        second.top() >= first.bottom(),
        "below: {first:?} {second:?}"
    );
}
