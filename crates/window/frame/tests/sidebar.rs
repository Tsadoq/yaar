//! Component test: the sidebar of the Frame on GPUI's test platform, with the Keeper played on the
//! other end of a socket pair.
//! What the sidebar shows is read back through the model it draws from.

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

fn drawn_width(cx: &mut VisualTestContext) -> Pixels {
    drawn("sidebar", cx)
        .expect("the sidebar is drawn")
        .size
        .width
}

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
        agent: None,
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

fn click_workspace(workspace: &WorkspaceId, cx: &mut VisualTestContext) {
    let row = drawn(&format!("sidebar-workspace-{workspace}"), cx)
        .expect("the row of the workspace is drawn");
    cx.simulate_click(row.center(), Modifiers::none());
}

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

fn showing(
    listed: &[Repository],
    window_size: Size<Pixels>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    let (mut shown, mut cx) = open_frame(window_size, Vec::new(), cx);
    shown.keeper.list(listed, &mut cx);
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

    let listed = two_repositories();
    shown.keeper.tell(Fact::Workspaces(listed.clone()));
    let every_row = [
        repository_row(&listed[0]),
        workspace_row('1', "main", true),
        workspace_row('2', "feature/login", false),
        repository_row(&listed[1]),
        workspace_row('3', "trunk", false),
        workspace_row('4', "detached at 0123456", false),
    ];
    wait_until(&mut cx, "both repositories in the sidebar", |cx| {
        shown.sidebar.read_with(cx, |sidebar, cx| sidebar.rows(cx)) == every_row
    });
    assert_drawn_as_listed(&shown.sidebar, &mut cx);

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
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);
    assert_drawn_as_listed(&shown.sidebar, &mut cx);

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
    // The width counts from the first frame: a window opened one pixel narrower than `RAIL_BELOW`
    // shows the rail without a resize.
    let (shown, mut cx) = showing(&two_repositories(), wide(RAIL_BELOW - px(1.)), cx);
    let sidebar = &shown.sidebar;
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    cx.simulate_resize(wide(RAIL_BELOW));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    cx.simulate_resize(wide(RAIL_BELOW - px(1.)));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    cx.simulate_resize(WIDE);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    cx.simulate_resize(wide(px(600.)));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Rail);
    assert_eq!(drawn_width(&mut cx), RAIL_WIDTH);

    cx.simulate_resize(wide(px(1600.)));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

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
    assert_eq!(pinned(sidebar, &mut cx), None);
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);

    for (pins, width) in [
        (SidebarForm::Rail, RAIL_WIDTH),
        (SidebarForm::Full, FULL_WIDTH),
    ] {
        cx.simulate_keystrokes(key_of::<ToggleSidebar>());
        assert_eq!(pinned(sidebar, &mut cx), Some(pins));
        assert_eq!(form(sidebar, &mut cx), pins);
        assert_eq!(drawn_width(&mut cx), width);

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

    cx.simulate_resize(wide(px(700.)));
    click_pin(&mut cx);
    assert_eq!(pinned(sidebar, &mut cx), Some(SidebarForm::Full));
    assert_eq!(form(sidebar, &mut cx), SidebarForm::Full);
    assert_eq!(drawn_width(&mut cx), FULL_WIDTH);
}

// The two tests below cover the case where the form on screen is not the pinned one: a rail
// by the width alone. A toggle that went by what is pinned would pin the rail there, and
// nothing visible would change.

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

fn with_a_worktree_that_is_gone() -> Vec<Repository> {
    vec![repository(
        'a',
        vec![
            workspace('1', "main"),
            Workspace {
                id: id('2'),
                path: "/gits/repository-a-removed".into(),
                head: None,
            },
        ],
    )]
}

#[gpui_kit::test]
fn a_worktree_that_is_gone_is_named_by_its_folder_marked_as_gone_and_still_selected_by_a_click(
    cx: &mut TestAppContext,
) {
    let listed = with_a_worktree_that_is_gone();
    for (width, form) in [
        (WIDE, SidebarForm::Full),
        (wide(RAIL_BELOW - px(1.)), SidebarForm::Rail),
    ] {
        let (mut shown, mut cx) = showing(&listed, width, cx);
        assert_eq!(self::form(&shown.sidebar, &mut cx), form);
        assert_eq!(
            rows(&shown.sidebar, &mut cx),
            [
                repository_row(&listed[0]),
                workspace_row('1', "main", true),
                SidebarRow::Workspace {
                    id: id('2'),
                    name: "repository-a-removed".to_owned(),
                    selected: false,
                    gone: true,
                    agent: None,
                },
            ]
        );
        let row = drawn(&format!("sidebar-workspace-{}", id('2')), &mut cx)
            .expect("the row of the worktree that is gone is drawn");
        let mark = drawn(&format!("sidebar-gone-{}", id('2')), &mut cx)
            .expect("the mark of the worktree that is gone is drawn");
        assert!(
            row.left() <= mark.left()
                && mark.right() <= row.right()
                && row.top() <= mark.top()
                && mark.bottom() <= row.bottom(),
            "the mark is at {mark:?}, outside its row at {row:?}"
        );
        assert!(
            drawn(&format!("sidebar-gone-{}", id('1')), &mut cx).is_none(),
            "a worktree that is there carries no mark"
        );
        assert_drawn_as_listed(&shown.sidebar, &mut cx);

        shown
            .keeper
            .expect(Request::GetChanges { workspace: id('1') }, &mut cx);
        click_workspace(&id('2'), &mut cx);
        assert_eq!(marked(&shown.sidebar, &mut cx), Some(id('2')));
        shown
            .keeper
            .expect(Request::GetChanges { workspace: id('2') }, &mut cx);
    }
}

#[gpui_kit::test]
fn the_add_control_opens_a_menu_whose_items_prompt_for_a_folder_or_start_a_new_worktree(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    assert_eq!(drawn("sidebar-add-menu", &mut cx), None);

    let add = drawn("sidebar-add", &mut cx).expect("the add control is drawn");
    cx.simulate_click(add.center(), Modifiers::none());
    assert!(drawn("sidebar-add-menu", &mut cx).is_some());

    let open = drawn("sidebar-add-open", &mut cx).expect("the open folder item is drawn");
    cx.simulate_click(open.center(), Modifiers::none());
    assert_eq!(drawn("sidebar-add-menu", &mut cx), None);
    assert!(cx.did_prompt_for_paths(), "open folder prompted for a path");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();

    let add = drawn("sidebar-add", &mut cx).expect("the add control is drawn");
    cx.simulate_click(add.center(), Modifiers::none());
    let worktree = drawn("sidebar-add-worktree", &mut cx).expect("the new worktree item is drawn");
    cx.simulate_click(worktree.center(), Modifiers::none());
    assert_eq!(drawn("sidebar-add-menu", &mut cx), None);
    // No agent CLI is staged here, so dispatching NewWorktree surfaces the notice instead of a field.
    assert!(
        shown
            .frame
            .read_with(&cx, |frame, cx| frame.notice(cx))
            .is_some(),
        "new worktree was dispatched"
    );
}

#[gpui_kit::test]
fn the_settings_button_in_the_sidebar_footer_opens_and_closes_settings_in_both_full_and_rail_forms(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = showing(&two_repositories(), WIDE, cx);
    let screen_open = |cx: &mut VisualTestContext| {
        shown
            .frame
            .read_with(cx, |frame, _| frame.settings_screen().is_some())
    };
    let click_settings = |cx: &mut VisualTestContext| {
        let button = drawn("sidebar-settings", cx).expect("the settings button is drawn");
        let sidebar = drawn("sidebar", cx).expect("the sidebar is drawn");
        assert!(
            sidebar.left() <= button.left()
                && button.right() <= sidebar.right()
                && sidebar.top() <= button.top()
                && button.bottom() <= sidebar.bottom(),
            "settings button {button:?} lies inside the sidebar {sidebar:?}"
        );
        cx.simulate_click(button.center(), Modifiers::none());
    };

    assert_eq!(form(&shown.sidebar, &mut cx), SidebarForm::Full);
    assert!(!screen_open(&mut cx));
    click_settings(&mut cx);
    assert!(screen_open(&mut cx), "full form click opens settings");
    assert!(drawn("settings-screen", &mut cx).is_some());
    click_settings(&mut cx);
    assert!(
        !screen_open(&mut cx),
        "full form second click closes settings"
    );
    assert_eq!(drawn("settings-screen", &mut cx), None);

    cx.simulate_resize(wide(RAIL_BELOW - px(1.)));
    assert_eq!(form(&shown.sidebar, &mut cx), SidebarForm::Rail);
    click_settings(&mut cx);
    assert!(screen_open(&mut cx), "rail form click opens settings");
    assert!(drawn("settings-screen", &mut cx).is_some());
    click_settings(&mut cx);
    assert!(
        !screen_open(&mut cx),
        "rail form second click closes settings"
    );
    assert_eq!(drawn("settings-screen", &mut cx), None);
}
