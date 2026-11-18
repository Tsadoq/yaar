//! Component test: the Frame with its sidebar and panes, on GPUI's test platform. The
//! keys of the Frame are bound from its table and pressed as a user presses them; what
//! they set off is read on the other end of a socket pair, where the test plays the
//! Keeper, and through what the Frame and the shared models show.

mod common;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    AppContext, Context, FocusHandle, InteractiveElement, IntoElement, Modifiers, Pixels, Render,
    Size, Styled, TestAppContext, VisualTestContext, Window, div, px,
};
use yaar_frame::{
    BuildPane, FULL_WIDTH, Frame, KeeperLink, OpenFolder, Pane, RAIL_WIDTH, SelectNextWorkspace,
    SelectPreviousWorkspace, SetBase, SidebarForm, ToggleSidebar,
};
use yaar_link::{Refusal, Reply, Repository, Request, WorkspaceId};

use common::{Shown, drawn, id, key_of, open_frame, repository, wait_until, workspace};

const WINDOW: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

/// One repository with two worktrees.
fn one_repository() -> Vec<Repository> {
    vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )]
}

fn get_changes(digit: char) -> Request {
    Request::GetChanges {
        workspace: id(digit),
    }
}

/// A window with these panes whose Keeper has answered the first question for the list
/// with `listed`, and has been asked for the changes of the first workspace of it.
fn showing(
    listed: &[Repository],
    panes: Vec<BuildPane>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    let (mut shown, mut cx) = open_frame(WINDOW, panes, cx);
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

fn on_screen(shown: &Shown, cx: &mut VisualTestContext) -> Option<WorkspaceId> {
    shown
        .models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
}

fn notice(shown: &Shown, cx: &mut VisualTestContext) -> Option<String> {
    shown.frame.read_with(cx, |frame, cx| frame.notice(cx))
}

fn pinned(shown: &Shown, cx: &mut VisualTestContext) -> Option<SidebarForm> {
    shown.sidebar.read_with(cx, |sidebar, _| sidebar.pinned())
}

/// Whether the field for the base is drawn.
fn field_is_open(cx: &mut VisualTestContext) -> bool {
    drawn("frame-base-field", cx).is_some()
}

/// A pane that shows nothing, drawn under the selector `pane-<name>`.
struct Blank {
    name: &'static str,
    focus: FocusHandle,
}

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = self.name;
        div()
            .track_focus(&self.focus)
            .debug_selector(move || format!("pane-{name}"))
            .size_full()
    }
}

fn blank(name: &'static str) -> BuildPane {
    Box::new(move |_, _, cx| {
        let focus = cx.focus_handle();
        let view = cx.new(|_| Blank {
            name,
            focus: focus.clone(),
        });
        Pane {
            name,
            view: view.into(),
            focus,
        }
    })
}

#[gpui_kit::test]
fn the_key_of_open_folder_asks_for_a_folder_and_sends_the_one_chosen(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = showing(&one_repository(), Vec::new(), cx);
    assert!(!cx.did_prompt_for_paths());

    // Given up in the dialog: nothing is asked of the Keeper. The request read further
    // down is the next one the Window sends, so nothing went out here.
    cx.simulate_keystrokes(key_of::<OpenFolder>());
    assert!(cx.did_prompt_for_paths(), "the folder dialog is open");
    cx.simulate_path_prompt_response(|asked| {
        assert!(asked.directories, "the dialog offers folders");
        assert!(!asked.files, "and no files");
        assert!(!asked.multiple, "one folder at a time");
        None
    });
    cx.run_until_parked();
    assert_eq!(notice(&shown, &mut cx), None);

    cx.simulate_keystrokes(key_of::<OpenFolder>());
    let folder = PathBuf::from("/gits/opened");
    cx.simulate_path_prompt_response(|_| Some(vec![folder.clone()]));
    let asked = shown.keeper.expect(
        Request::OpenFolder {
            path: folder.clone(),
        },
        &mut cx,
    );

    // Opened: its workspace comes on screen with the list that holds it.
    let opened = repository('b', vec![workspace('3', "opened")]);
    shown.keeper.reply(
        asked,
        Ok(Reply::Opened {
            repository: opened.id.clone(),
            workspace: id('3'),
        }),
    );
    let mut listed = one_repository();
    listed.push(opened);
    shown.keeper.tell(yaar_link::Fact::Workspaces(listed));
    wait_until(&mut cx, "the opened workspace on screen", |cx| {
        shown
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected() == Some(&id('3')))
    });
    assert_eq!(notice(&shown, &mut cx), None);
}

#[gpui_kit::test]
fn a_refusal_of_the_keeper_is_shown_until_the_user_asks_again(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = showing(&one_repository(), Vec::new(), cx);
    assert_eq!(notice(&shown, &mut cx), None);
    assert_eq!(drawn("frame-notice", &mut cx), None);

    cx.simulate_keystrokes(key_of::<OpenFolder>());
    let folder = PathBuf::from("/tmp/plain folder");
    cx.simulate_path_prompt_response(|_| Some(vec![folder.clone()]));
    let asked = shown.keeper.expect(
        Request::OpenFolder {
            path: folder.clone(),
        },
        &mut cx,
    );
    let refusal = Refusal::NotARepository { path: folder };
    shown.keeper.reply(asked, Err(refusal.clone()));

    // The reason, in the Keeper's own words, and drawn.
    wait_until(&mut cx, "the notice", |cx| {
        shown.frame.read_with(cx, |frame, cx| frame.notice(cx)) == Some(refusal.to_string())
    });
    assert_eq!(
        notice(&shown, &mut cx).as_deref(),
        Some("/tmp/plain folder holds no git repository and no worktree")
    );
    assert!(
        drawn("frame-notice", &mut cx).is_some(),
        "the notice is drawn"
    );
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));

    // Asking for the next thing takes the notice away, before any answer.
    cx.simulate_keystrokes(key_of::<OpenFolder>());
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/gits/other")]));
    shown.keeper.expect(
        Request::OpenFolder {
            path: PathBuf::from("/gits/other"),
        },
        &mut cx,
    );
    assert_eq!(notice(&shown, &mut cx), None);
    assert_eq!(drawn("frame-notice", &mut cx), None);
}

#[gpui_kit::test]
fn the_key_of_set_base_opens_a_field_and_enter_sends_what_was_typed(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = showing(&one_repository(), Vec::new(), cx);
    assert!(!field_is_open(&mut cx));

    cx.simulate_keystrokes(key_of::<SetBase>());
    assert!(field_is_open(&mut cx));
    // The keys typed next go to the field, also those that are keys of a pane elsewhere.
    cx.simulate_input("origin/jks");
    cx.simulate_keystrokes("enter");
    let asked = shown.keeper.expect(
        Request::SetBase {
            workspace: id('1'),
            base: "origin/jks".to_owned(),
        },
        &mut cx,
    );
    assert!(!field_is_open(&mut cx), "Enter closes the field");

    // The keys of the Frame work again where the field was.
    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(pinned(&shown, &mut cx), Some(SidebarForm::Rail));

    // A base git does not know: the Keeper says so, and the Frame shows it.
    let refusal = Refusal::UnknownRevision {
        revision: "origin/jks".to_owned(),
    };
    shown.keeper.reply(asked, Err(refusal.clone()));
    wait_until(&mut cx, "the notice", |cx| {
        shown.frame.read_with(cx, |frame, cx| frame.notice(cx)) == Some(refusal.to_string())
    });

    // The base is that of the workspace on screen when Enter is pressed, and a field
    // opened again is empty.
    cx.simulate_keystrokes(key_of::<SelectNextWorkspace>());
    cx.simulate_keystrokes(key_of::<SetBase>());
    shown.keeper.expect(get_changes('2'), &mut cx);
    assert!(field_is_open(&mut cx));
    assert_eq!(notice(&shown, &mut cx), Some(refusal.to_string()));
    cx.simulate_input(" main ");
    cx.simulate_keystrokes("enter");
    shown.keeper.expect(
        Request::SetBase {
            workspace: id('2'),
            // The blanks around the name are not part of it.
            base: "main".to_owned(),
        },
        &mut cx,
    );
    assert_eq!(notice(&shown, &mut cx), None, "asked again");
}

#[gpui_kit::test]
fn a_field_given_up_or_left_empty_sets_no_base(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = showing(&one_repository(), Vec::new(), cx);

    // Escape gives up: the field closes and what was typed is gone.
    cx.simulate_keystrokes(key_of::<SetBase>());
    cx.simulate_input("givenup");
    cx.simulate_keystrokes("escape");
    assert!(!field_is_open(&mut cx), "Escape closes the field");
    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(
        pinned(&shown, &mut cx),
        Some(SidebarForm::Rail),
        "the keys of the Frame work after Escape"
    );

    // Enter in an empty field, or in one that holds blanks only, asks nothing and
    // leaves the field open.
    cx.simulate_keystrokes(key_of::<SetBase>());
    cx.simulate_keystrokes("enter");
    assert!(field_is_open(&mut cx));
    cx.simulate_input("  ");
    cx.simulate_keystrokes("enter");
    assert!(field_is_open(&mut cx));

    // The key again while the field is open leaves it as it is.
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("ma");
    cx.simulate_keystrokes(key_of::<SetBase>());
    cx.simulate_input("in");
    cx.simulate_keystrokes("enter");

    // The first base the Keeper hears of is the last one typed: nothing was sent for
    // the field given up or for the empty ones.
    shown.keeper.expect(
        Request::SetBase {
            workspace: id('1'),
            base: "main".to_owned(),
        },
        &mut cx,
    );
}

#[gpui_kit::test]
fn with_no_workspace_on_screen_the_key_of_set_base_opens_no_field(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = showing(&[], Vec::new(), cx);
    assert_eq!(on_screen(&shown, &mut cx), None);

    cx.simulate_keystrokes(key_of::<SetBase>());
    assert!(!field_is_open(&mut cx));
    cx.simulate_input("main");
    cx.simulate_keystrokes("enter");

    // The next thing the Keeper hears is the folder opened afterwards.
    cx.simulate_keystrokes(key_of::<OpenFolder>());
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/gits/first")]));
    shown.keeper.expect(
        Request::OpenFolder {
            path: PathBuf::from("/gits/first"),
        },
        &mut cx,
    );
}

#[gpui_kit::test]
fn the_keys_of_next_and_previous_workspace_change_the_workspace_on_screen(cx: &mut TestAppContext) {
    let listed = [
        repository('a', vec![workspace('1', "main"), workspace('2', "feature")]),
        repository('b', vec![workspace('3', "main")]),
    ];
    let (mut shown, mut cx) = showing(&listed, Vec::new(), cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('1')));

    cx.simulate_keystrokes(key_of::<SelectNextWorkspace>());
    assert_eq!(on_screen(&shown, &mut cx), Some(id('2')));
    shown.keeper.expect(get_changes('2'), &mut cx);
    cx.simulate_keystrokes(key_of::<SelectNextWorkspace>());
    assert_eq!(on_screen(&shown, &mut cx), Some(id('3')));
    shown.keeper.expect(get_changes('3'), &mut cx);

    cx.simulate_keystrokes(key_of::<SelectPreviousWorkspace>());
    assert_eq!(on_screen(&shown, &mut cx), Some(id('2')));
    shown.keeper.expect(get_changes('2'), &mut cx);
}

#[gpui_kit::test]
fn when_the_window_comes_to_the_front_the_list_and_the_changes_are_asked_for_again(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = showing(&one_repository(), Vec::new(), cx);
    let at_the_front =
        |cx: &mut VisualTestContext| cx.update(|window, _| window.is_window_active());
    // A user's system opens a window at the front; the test platform leaves that to the
    // test.
    assert!(!at_the_front(&mut cx));
    cx.update(|window, _| window.activate_window());
    shown.keeper.expect(Request::ListWorkspaces, &mut cx);
    shown.keeper.expect(get_changes('1'), &mut cx);
    assert!(at_the_front(&mut cx));

    // Leaving the front asks nothing: what is read after it is what coming back asked.
    cx.deactivate_window();
    assert!(!at_the_front(&mut cx));
    cx.update(|window, _| window.activate_window());
    shown.keeper.expect(Request::ListWorkspaces, &mut cx);
    shown.keeper.expect(get_changes('1'), &mut cx);
}

#[gpui_kit::test]
fn the_panes_lie_beside_the_sidebar_in_the_order_given_and_the_keys_of_the_frame_work_in_them(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&one_repository(), vec![blank("left"), blank("right")], cx);
    assert_eq!(
        shown.frame.read_with(&cx, |frame, _| frame.panes()),
        ["left", "right"]
    );

    let pane = |name: &str, cx: &mut VisualTestContext| {
        drawn(&format!("pane-{name}"), cx).expect("the pane is drawn")
    };
    let (left, right) = (pane("left", &mut cx), pane("right", &mut cx));
    assert_eq!(
        left.left(),
        FULL_WIDTH,
        "the first pane starts where the sidebar ends"
    );
    assert_eq!(right.left(), left.right(), "the second follows the first");
    assert_eq!(right.right(), WINDOW.width, "and ends with the window");
    assert_eq!(left.size.width, right.size.width);
    assert_eq!(left.bottom(), WINDOW.height);

    // The focus is in a pane; the keys of the Frame work from there.
    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(pinned(&shown, &mut cx), Some(SidebarForm::Rail));
    assert_eq!(
        pane("left", &mut cx).left(),
        RAIL_WIDTH,
        "the panes take the room the rail leaves"
    );
    assert_eq!(pane("right", &mut cx).right(), WINDOW.width);
}

/// The handles the panes of a window gave the Frame, in the order of the panes.
type Kept = Rc<RefCell<Vec<FocusHandle>>>;

/// A blank pane whose builder keeps the handle it gave the Frame, as a pane does.
fn keeping(name: &'static str, kept: &Kept) -> BuildPane {
    let kept = kept.clone();
    Box::new(move |models, window, cx| {
        let pane = blank(name)(models, window, cx);
        kept.borrow_mut().push(pane.focus.clone());
        pane
    })
}

/// For each pane, whether the focus is on the handle it gave the Frame.
fn focused(kept: &Kept, cx: &mut VisualTestContext) -> Vec<bool> {
    cx.update(|window, _| {
        kept.borrow()
            .iter()
            .map(|focus| focus.is_focused(window))
            .collect()
    })
}

#[gpui_kit::test]
fn the_first_pane_has_the_focus_when_the_window_opens(cx: &mut TestAppContext) {
    let kept = Kept::default();
    let (_shown, mut cx) = showing(
        &one_repository(),
        vec![keeping("left", &kept), keeping("right", &kept)],
        cx,
    );
    assert_eq!(focused(&kept, &mut cx), [true, false]);
}

#[gpui_kit::test]
fn a_click_on_what_the_frame_draws_around_the_panes_leaves_the_focus_in_the_pane(
    cx: &mut TestAppContext,
) {
    let kept = Kept::default();
    let (mut shown, mut cx) = showing(
        &one_repository(),
        vec![keeping("left", &kept), keeping("right", &kept)],
        cx,
    );
    let click = |selector: &str, cx: &mut VisualTestContext| {
        let place = drawn(selector, cx).unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(place.center(), Modifiers::none());
    };

    // The user works in the second pane.
    click("pane-right", &mut cx);
    assert_eq!(focused(&kept, &mut cx), [false, true]);

    // A workspace picked in the sidebar: the click does its work, and the keys of the
    // pane go on working without a click into the pane.
    click(&format!("sidebar-workspace-{}", id('2')), &mut cx);
    assert_eq!(on_screen(&shown, &mut cx), Some(id('2')));
    shown.keeper.expect(get_changes('2'), &mut cx);
    assert_eq!(focused(&kept, &mut cx), [false, true]);

    // The same after the pin control,
    click("sidebar-pin", &mut cx);
    assert_eq!(pinned(&shown, &mut cx), Some(SidebarForm::Rail));
    assert_eq!(focused(&kept, &mut cx), [false, true]);

    // after the line of a repository, which takes no click,
    click("sidebar-pin", &mut cx);
    assert_eq!(pinned(&shown, &mut cx), Some(SidebarForm::Full));
    let repository = &one_repository()[0];
    click(&format!("sidebar-repository-{}", repository.id), &mut cx);
    assert_eq!(focused(&kept, &mut cx), [false, true]);

    // and after the notice.
    cx.simulate_keystrokes(key_of::<OpenFolder>());
    let folder = PathBuf::from("/tmp/plain folder");
    cx.simulate_path_prompt_response(|_| Some(vec![folder.clone()]));
    let asked = shown.keeper.expect(
        Request::OpenFolder {
            path: folder.clone(),
        },
        &mut cx,
    );
    shown
        .keeper
        .reply(asked, Err(Refusal::NotARepository { path: folder }));
    wait_until(&mut cx, "the notice", |cx| {
        shown
            .frame
            .read_with(cx, |frame, cx| frame.notice(cx))
            .is_some()
    });
    click("frame-notice", &mut cx);
    assert_eq!(focused(&kept, &mut cx), [false, true]);

    // A click into the other pane still moves the focus there.
    click("pane-left", &mut cx);
    assert_eq!(focused(&kept, &mut cx), [true, false]);
}

/// The view of a pane that draws the place of its focus only while `here` says so.
struct Leaving {
    focus: FocusHandle,
    here: Rc<Cell<bool>>,
}

impl Render for Leaving {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pane = div().size_full();
        if self.here.get() {
            pane.track_focus(&self.focus)
        } else {
            pane
        }
    }
}

/// A pane that draws the place of its focus only while `here` says so.
fn leaving(here: &Rc<Cell<bool>>) -> BuildPane {
    let here = here.clone();
    Box::new(move |_, _, cx| {
        let focus = cx.focus_handle();
        let view = cx.new(|_| Leaving {
            focus: focus.clone(),
            here,
        });
        Pane {
            name: "leaving",
            view: view.into(),
            focus,
        }
    })
}

#[gpui_kit::test]
fn the_keys_of_the_frame_work_after_what_had_the_focus_left_the_window(cx: &mut TestAppContext) {
    let here = Rc::new(Cell::new(true));
    let (shown, mut cx) = showing(&one_repository(), vec![leaving(&here)], cx);

    // What has the focus is drawn no more, as a row of a pane that went away.
    here.set(false);
    cx.update(|window, _| window.refresh());

    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(pinned(&shown, &mut cx), Some(SidebarForm::Rail));
}

#[gpui_kit::test]
fn the_keys_of_the_frame_work_when_the_first_pane_never_draws_where_its_focus_is(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(
        &one_repository(),
        vec![leaving(&Rc::new(Cell::new(false)))],
        cx,
    );
    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(pinned(&shown, &mut cx), Some(SidebarForm::Rail));
}

#[gpui_kit::test]
fn a_frame_made_for_the_link_alone_has_no_sidebar_no_pane_and_no_notice(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let frame = cx.new(|cx| Frame::new(link, cx));
    frame.read_with(cx, |frame, cx| {
        assert!(frame.sidebar().is_none());
        assert!(frame.panes().is_empty());
        assert_eq!(frame.notice(cx), None);
    });
}
