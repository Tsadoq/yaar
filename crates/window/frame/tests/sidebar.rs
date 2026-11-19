//! Component test: the sidebar of the Frame, on GPUI's test platform with no window on
//! any screen. Keys, clicks and the wheel go in the way a user's do; what the sidebar
//! shows is read back through the model it draws from. The test plays the Keeper on the
//! other end of a socket pair.

mod common;

use gpui_kit::{
    AppContext, Bounds, Entity, Modifiers, Pixels, ScrollDelta, ScrollEvent, Size, TestAppContext,
    VisualTestContext, point, px, size,
};
use yaar_frame::{
    FULL_WIDTH, KeeperLink, RAIL_BELOW, RAIL_WIDTH, Sidebar, SidebarForm, SidebarRow,
    ToggleSidebar, UnpinSidebar, WorkspaceList,
};
use yaar_link::{Fact, Head, Repository, Request, Workspace, WorkspaceId};

use common::{Shown, drawn, id, key_of, open_frame, repository, wait_until, workspace};

/// The size the app's window opens at: wide enough for the full list.
const WIDE: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

/// A window at this width.
fn wide(width: Pixels) -> Size<Pixels> {
    size(width, px(800.))
}

fn rows(sidebar: &Entity<Sidebar>, cx: &mut VisualTestContext) -> Vec<SidebarRow> {
    sidebar.read_with(cx, |sidebar, cx| sidebar.rows(cx))
}

fn form(sidebar: &Entity<Sidebar>, cx: &mut VisualTestContext) -> SidebarForm {
    sidebar.read_with(cx, |sidebar, _| sidebar.form())
}

fn pinned(sidebar: &Entity<Sidebar>, cx: &mut VisualTestContext) -> Option<SidebarForm> {
    sidebar.read_with(cx, |sidebar, _| sidebar.pinned())
}

/// How wide the sidebar was drawn in the last frame.
fn drawn_width(cx: &mut VisualTestContext) -> Pixels {
    drawn("sidebar", cx)
        .expect("the sidebar is drawn")
        .size
        .width
}

/// Holds what is drawn to what [`Sidebar::rows`] lists: every row of the list is drawn,
/// inside the sidebar, and each one under the one the list has before it.
fn assert_drawn_as_listed(sidebar: &Entity<Sidebar>, cx: &mut VisualTestContext) {
    let whole = drawn("sidebar", cx).expect("the sidebar is drawn");
    let mut above: Option<(SidebarRow, Bounds<Pixels>)> = None;
    for row in rows(sidebar, cx) {
        let selector = match &row {
            SidebarRow::Repository { id, .. } => format!("sidebar-repository-{id}"),
            SidebarRow::Workspace { id, .. } => format!("sidebar-workspace-{id}"),
        };
        let Some(place) = drawn(&selector, cx) else {
            panic!("{row:?} is listed and not drawn");
        };
        assert!(
            place.size.width > px(0.) && place.size.height > px(0.),
            "{row:?} is drawn with no room: {place:?}"
        );
        assert!(
            whole.left() <= place.left()
                && place.right() <= whole.right()
                && whole.top() <= place.top()
                && place.bottom() <= whole.bottom(),
            "{row:?} is drawn at {place:?}, outside the sidebar at {whole:?}"
        );
        if let Some((above, ends)) = &above {
            assert!(
                ends.bottom() <= place.top(),
                "{row:?} starts at {:?}, which is not under {above:?}, which ends at {:?}",
                place.top(),
                ends.bottom()
            );
        }
        above = Some((row, place));
    }
}

fn repository_row(repository: &Repository) -> SidebarRow {
    SidebarRow::Repository {
        id: repository.id.clone(),
        name: repository.name.clone(),
    }
}

fn workspace_row(digit: char, name: &str, selected: bool) -> SidebarRow {
    SidebarRow::Workspace {
        id: id(digit),
        name: name.to_owned(),
        selected,
        gone: false,
    }
}

/// The workspace the sidebar marks as on screen; fails when it marks more than one.
fn marked(sidebar: &Entity<Sidebar>, cx: &mut VisualTestContext) -> Option<WorkspaceId> {
    let mut marked = rows(sidebar, cx).into_iter().filter_map(|row| match row {
        SidebarRow::Workspace {
            id, selected: true, ..
        } => Some(id),
        _ => None,
    });
    let first = marked.next();
    assert_eq!(marked.next(), None, "one workspace is on screen, not two");
    first
}

/// Clicks the row of a workspace where it was drawn.
fn click_workspace(workspace: &WorkspaceId, cx: &mut VisualTestContext) {
    let row = drawn(&format!("sidebar-workspace-{workspace}"), cx)
        .expect("the row of the workspace is drawn");
    cx.simulate_click(row.center(), Modifiers::none());
}

/// Two repositories: one with two worktrees, one with a worktree on a branch and one on
/// no branch.
fn two_repositories() -> Vec<Repository> {
    vec![
        repository(
            'a',
            vec![workspace('1', "main"), workspace('2', "feature/login")],
        ),
        repository(
            'b',
            vec![
                workspace('3', "trunk"),
                Workspace {
                    id: id('4'),
                    path: "/gits/elsewhere".into(),
                    head: Some(Head::Detached(
                        "0123456789abcdef0123456789abcdef01234567".to_owned(),
                    )),
                },
            ],
        ),
    ]
}

/// A window whose Keeper has answered the first question for the list with `listed`, and
/// whose sidebar shows it.
fn showing(
    listed: &[Repository],
    window_size: Size<Pixels>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    let (mut shown, mut cx) = open_frame(window_size, Vec::new(), cx);
    shown.keeper.list(listed, &mut cx);
    // One row for each repository and one for each of its worktrees.
    let every_row = listed
        .iter()
        .map(|repository| 1 + repository.workspaces.len())
        .sum::<usize>();
    wait_until(&mut cx, "the sidebar to show the list", |cx| {
        shown
            .sidebar
            .read_with(cx, |sidebar, cx| sidebar.rows(cx))
            .len()
            == every_row
    });
    (shown, cx)
}

#[gpui_kit::test]
fn the_sidebar_lists_each_repository_with_its_worktrees_under_it_each_named_by_its_branch(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = open_frame(WIDE, Vec::new(), cx);
    assert_eq!(rows(&shown.sidebar, &mut cx), []);

    // What the Keeper answers when the window opens: one repository with one worktree.
    let first = repository('a', vec![workspace('1', "main")]);
    shown.keeper.list(std::slice::from_ref(&first), &mut cx);
    wait_until(&mut cx, "the first repository in the sidebar", |cx| {
        shown.sidebar.read_with(cx, |sidebar, cx| sidebar.rows(cx))
            == [repository_row(&first), workspace_row('1', "main", true)]
    });
    shown
        .keeper
        .expect(Request::GetChanges { workspace: id('1') }, &mut cx);
    assert_drawn_as_listed(&shown.sidebar, &mut cx);

    // What it tells when a worktree was made there and a second repository was opened.
    let listed = two_repositories();
    shown.keeper.tell(Fact::Workspaces(listed.clone()));
    let every_row = [
        repository_row(&listed[0]),
        workspace_row('1', "main", true),
        workspace_row('2', "feature/login", false),
        repository_row(&listed[1]),
        workspace_row('3', "trunk", false),
        // No branch is checked out there: it is named by the commit it sits on.
        workspace_row('4', "detached at 0123456", false),
    ];
    wait_until(&mut cx, "both repositories in the sidebar", |cx| {
        shown.sidebar.read_with(cx, |sidebar, cx| sidebar.rows(cx)) == every_row
    });
    // What is drawn is that list: each repository above its own worktrees.
    assert_drawn_as_listed(&shown.sidebar, &mut cx);

    // A click on a worktree puts it on screen: the sidebar marks it, the shared list has
    // it selected, and its changes are asked for.
    click_workspace(&id('3'), &mut cx);
    assert_eq!(marked(&shown.sidebar, &mut cx), Some(id('3')));
    assert_eq!(
        shown
            .models
            .workspaces
            .read_with(&cx, |list, _| list.selected().cloned()),
        Some(id('3'))
    );
    shown
        .keeper
        .expect(Request::GetChanges { workspace: id('3') }, &mut cx);
    // Nothing else of the rows moved.
    let names = |rows: &[SidebarRow]| -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                SidebarRow::Repository { name, .. } | SidebarRow::Workspace { name, .. } => {
                    name.clone()
                }
            })
            .collect()
    };
    assert_eq!(names(&rows(&shown.sidebar, &mut cx)), names(&every_row));
    assert_drawn_as_listed(&shown.sidebar, &mut cx);
}

#[gpui_kit::test]
fn the_rail_holds_the_same_rows_and_a_click_on_a_mark_puts_its_workspace_on_screen(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    let as_full_list = rows(&shown.sidebar, &mut cx);

    cx.simulate_resize(wide(RAIL_BELOW - px(1.)));
    assert_eq!(form(&shown.sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(rows(&shown.sidebar, &mut cx), as_full_list);
    // Drawn narrower, and still each repository above its own worktrees.
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);
    assert_drawn_as_listed(&shown.sidebar, &mut cx);

    // The mark of a workspace lies inside the rail, and takes the click.
    let mark = drawn(&format!("sidebar-workspace-{}", id('2')), &mut cx)
        .expect("the mark of the workspace is drawn");
    assert!(
        mark.right() <= RAIL_WIDTH,
        "the mark ends at {:?}, outside the rail",
        mark.right()
    );
    click_workspace(&id('2'), &mut cx);
    assert_eq!(marked(&shown.sidebar, &mut cx), Some(id('2')));
}

#[gpui_kit::test]
fn the_wheel_brings_a_worktree_far_down_the_list_within_reach_of_a_click(cx: &mut TestAppContext) {
    // More worktrees than a low window has room for.
    let many: Vec<Workspace> = (0..60)
        .map(|n| Workspace {
            id: WorkspaceId::new(format!("{n:032x}")),
            path: format!("/gits/worktree-{n}").into(),
            head: Some(Head::Branch(format!("branch-{n}"))),
        })
        .collect();
    let last = many[59].id.clone();
    let window = size(px(1280.), px(300.));
    let (shown, mut cx) = showing(&[repository('a', many)], window, cx);

    let row = |cx: &mut VisualTestContext| {
        drawn(&format!("sidebar-workspace-{last}"), cx).expect("the last row is laid out")
    };
    assert!(
        row(&mut cx).top() >= window.height,
        "the last row starts below the window"
    );

    cx.simulate_event(ScrollEvent {
        position: point(FULL_WIDTH / 2., px(100.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-10_000.))),
        ..Default::default()
    });
    let row = row(&mut cx);
    assert!(
        row.bottom() <= window.height,
        "after the wheel the last row ends at {:?}, inside the window",
        row.bottom()
    );
    cx.simulate_click(row.center(), Modifiers::none());
    assert_eq!(marked(&shown.sidebar, &mut cx), Some(last));
}

#[gpui_kit::test]
fn narrower_than_the_set_width_with_nothing_pinned_the_sidebar_is_a_rail_and_wider_the_full_list(
    cx: &mut TestAppContext,
) {
    // The width counts from the first frame: a window opened one pixel narrower than the
    // set width shows the rail without a resize.
    let (shown, mut cx) = showing(&two_repositories(), wide(RAIL_BELOW - px(1.)), cx);
    let sidebar = &shown.sidebar;
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    // The set width itself is not narrower than the set width.
    cx.simulate_resize(wide(RAIL_BELOW));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    // One pixel narrower is, also when a resize brings the window there.
    cx.simulate_resize(wide(RAIL_BELOW - px(1.)));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    // The size the app's window opens at.
    cx.simulate_resize(WIDE);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    cx.simulate_resize(wide(px(600.)));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    cx.simulate_resize(wide(px(1600.)));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    // The width alone did all of it: nothing was pinned on the way.
    assert_eq!(pinned(sidebar, &mut cx), None);
}

#[gpui_kit::test]
fn a_window_opened_narrow_shows_the_rail_at_once(cx: &mut TestAppContext) {
    let (shown, mut cx) = showing(&two_repositories(), wide(px(900.)), cx);
    assert_eq!(pinned(&shown.sidebar, &mut cx), None);
    assert_eq!(form(&shown.sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);
}

#[gpui_kit::test]
fn a_window_opened_wide_shows_the_full_list_at_once(cx: &mut TestAppContext) {
    // The size the app's window opens at, with no resize since.
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    assert_eq!(pinned(&shown.sidebar, &mut cx), None);
    assert_eq!(form(&shown.sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);
}

#[gpui_kit::test]
fn pinned_as_a_rail_or_as_the_full_list_the_sidebar_keeps_that_form_at_any_width(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    let sidebar = &shown.sidebar;
    let narrow = wide(RAIL_BELOW - px(1.));
    let narrower = wide(px(500.));
    let wider = wide(px(2000.));
    // Opened wide with nothing pinned: the full list, by the width.
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    for (pins, width) in [
        // In a wide window the sidebar is the full list, so the key pins the rail; and
        // the other way round.
        (SidebarForm::Rail, RAIL_WIDTH),
        (SidebarForm::Full, FULL_WIDTH),
    ] {
        cx.simulate_keystrokes(key_of::<ToggleSidebar>());
        assert_eq!(pinned(sidebar, &mut cx), Some(pins));
        assert_eq!(form(sidebar, &mut cx), pins);
        assert_eq!(drawn_width(&mut cx), width);

        // On both sides of the set width, and back.
        for window in [narrow, narrower, wide(RAIL_BELOW), wider, WIDE] {
            cx.simulate_resize(window);
            assert_eq!(
                form(sidebar, &mut cx),
                pins,
                "pinned as {pins:?}, in a window {:?} wide",
                window.width
            );
            assert_eq!(drawn_width(&mut cx), width, "drawn as {pins:?}");
            assert_eq!(pinned(sidebar, &mut cx), Some(pins));
        }
    }

    // Unpinned in a narrow window, the width has its say again: at once, and from then on.
    cx.simulate_resize(narrow);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full, "still pinned");
    cx.simulate_keystrokes(key_of::<UnpinSidebar>());
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);
    cx.simulate_resize(WIDE);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);
}

#[gpui_kit::test]
fn a_click_on_the_pin_control_pins_the_form_the_sidebar_does_not_have(cx: &mut TestAppContext) {
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    let sidebar = &shown.sidebar;
    let click_pin = |cx: &mut VisualTestContext| {
        let pin = drawn("sidebar-pin", cx).expect("the pin control is drawn");
        cx.simulate_click(pin.center(), Modifiers::none());
    };

    click_pin(&mut cx);
    assert_eq!(pinned(sidebar, &mut cx), Some(SidebarForm::Rail));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);

    // The control is there in the rail too, and pins the full list in a window that is
    // too narrow for it by its width.
    cx.simulate_resize(wide(px(700.)));
    click_pin(&mut cx);
    assert_eq!(pinned(sidebar, &mut cx), Some(SidebarForm::Full));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);
}

// The two tests below hold the one case where the form on screen is not the pinned one:
// a rail by the width alone. A toggle that went by what is pinned would pin the rail
// there, and nothing the user sees would change.

#[gpui_kit::test]
fn the_key_pins_the_full_list_in_a_window_whose_width_alone_made_the_sidebar_a_rail(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    let sidebar = &shown.sidebar;
    cx.simulate_resize(wide(RAIL_BELOW - px(1.)));
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(pinned(sidebar, &mut cx), Some(SidebarForm::Full));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    // And back: every press changes what is seen.
    cx.simulate_keystrokes(key_of::<ToggleSidebar>());
    assert_eq!(pinned(sidebar, &mut cx), Some(SidebarForm::Rail));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);
}

#[gpui_kit::test]
fn a_click_on_the_pin_control_pins_the_full_list_in_a_window_opened_too_narrow_for_it(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&two_repositories(), wide(px(900.)), cx);
    let sidebar = &shown.sidebar;
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    let pin = drawn("sidebar-pin", &mut cx).expect("the pin control is drawn");
    cx.simulate_click(pin.center(), Modifiers::none());
    assert_eq!(pinned(sidebar, &mut cx), Some(SidebarForm::Full));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);
}

#[gpui_kit::test]
fn a_sidebar_in_no_frame_is_the_full_list_until_a_form_is_pinned(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let list = cx.new(|cx| WorkspaceList::new(link, cx));
    let sidebar = cx.new(|cx| Sidebar::new(list, cx));
    let state = |cx: &mut TestAppContext| {
        sidebar.read_with(cx, |sidebar, _| (sidebar.pinned(), sidebar.form()))
    };
    // It knows no window, so no width has a say.
    assert_eq!(state(cx), (None, SidebarForm::Full));

    sidebar.update(cx, |sidebar, cx| sidebar.pin(Some(SidebarForm::Rail), cx));
    assert_eq!(state(cx), (Some(SidebarForm::Rail), SidebarForm::Rail));
    sidebar.update(cx, |sidebar, cx| sidebar.pin(None, cx));
    assert_eq!(state(cx), (None, SidebarForm::Full));
}
