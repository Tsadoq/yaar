//! The Window program: registers the panes in the Frame and binds the keys.

mod appearance;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::theme::Theme;
use gpui_kit::*;
use yaar_frame::{
    Asks, BuildPane, CloseWindow, Control, HideApp, HideOtherApps, Minimize, Models, Navigation,
    Opened, PROGRAM_CONTEXT, PaneEntry, Place,
};
use yaar_navigator::{Navigator, STOP_DEADLINE};

/// Quits the Window, the plain way: the Keeper and what it holds stay.
pub use yaar_frame::Quit;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn panes() -> Vec<PaneEntry> {
    vec![yaar_review_pane::entry(), yaar_terminal_pane::entry()]
}

/// The controls whose key works while a terminal has the focus; other keys reach its program.
pub fn kept_keys() -> Vec<Control> {
    controls()
        .into_iter()
        .filter(|control| match control.place {
            Place::Kept => true,
            Place::In(context) => context == PROGRAM_CONTEXT,
            // An Anywhere key is bound with no context only where it is a Cmd key (macOS); off
            // macOS it belongs to the program in a terminal.
            Place::Anywhere => control.context().is_none(),
        })
        .collect()
}

/// Each item runs the action of the `controls` row it is titled for. GPUI's Linux platforms show no
/// menus.
pub fn menus() -> Vec<Menu> {
    let table = controls();
    // A title with no row on this system (Hide off macOS) is left out, and a group left empty with
    // it.
    let layout: [(&str, &[&[&str]]); 3] = [
        (
            "YAAR",
            &[
                &["Settings…"],
                &["Hide YAAR", "Hide Others"],
                &["Quit", "Quit and Stop Agents"],
            ],
        ),
        (
            "File",
            &[
                &["New Worktree with Agent…", "Open Folder…"],
                &["Close Window"],
            ],
        ),
        ("Window", &[&["Minimize"]]),
    ];
    layout
        .into_iter()
        .map(|(name, groups)| {
            let mut items = Vec::new();
            for group in groups {
                let group: Vec<MenuItem> = group
                    .iter()
                    .filter_map(|title| {
                        let row = table.iter().find(|control| control.title == *title)?;
                        Some(MenuItem::Action {
                            name: row.title.into(),
                            action: (row.action)(),
                            os_action: None,
                            checked: false,
                            disabled: false,
                        })
                    })
                    .collect();
                if !group.is_empty() {
                    if !items.is_empty() {
                        items.push(MenuItem::Separator);
                    }
                    items.extend(group);
                }
            }
            Menu::new(name).items(items)
        })
        .collect()
}

pub fn controls() -> Vec<Control> {
    let mut controls = yaar_frame::controls();
    controls.extend(yaar_frame::settings_screen::controls());
    for pane in panes() {
        controls.extend((pane.controls)());
    }
    controls.extend(own_controls());
    controls
}

/// Each is a Cmd key where it works at all, so a terminal does not take it from the app.
fn own_controls() -> Vec<Control> {
    let window = |title, action: fn() -> Box<dyn Action>, keystroke| Control {
        title,
        action,
        keystroke,
        place: Place::Anywhere,
        sends: None,
        asks: Asks::Nothing,
    };
    let mut rows = vec![window(
        "Close Window",
        || Box::new(CloseWindow),
        "secondary-w",
    )];
    // Hiding and minimising are the system's on macOS; elsewhere the window manager has keys.
    if cfg!(target_os = "macos") {
        rows.extend([
            window("Hide YAAR", || Box::new(HideApp), "secondary-h"),
            window("Hide Others", || Box::new(HideOtherApps), "secondary-alt-h"),
            window("Minimize", || Box::new(Minimize), "secondary-m"),
        ]);
    }
    rows
}

pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    yaar_ui::init(cx);
    // After the toolkit, whose bindings it has to outrank.
    yaar_frame::release_program_keys(cx);
    Theme::sync_system_appearance(None, cx);
    yaar_frame::bind(&controls(), cx);
    cx.set_menus(menus());
    cx.on_action(|_: &Quit, cx| cx.quit());
    handle_window_actions(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// A menu item runs the action with no element to start from, so the focused window is asked of the
/// platform.
fn handle_window_actions(cx: &mut App) {
    cx.on_action(|_: &CloseWindow, cx| {
        with_active_window(cx, |window| window.remove_window());
    });
    if cfg!(target_os = "macos") {
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
        cx.on_action(|_: &Minimize, cx| {
            with_active_window(cx, |window| window.minimize_window());
        });
    }
}

/// An action handler runs while the window it came from is being updated, so the work on the
/// active window is deferred to the end of the current effects.
fn with_active_window(cx: &mut App, then: impl FnOnce(&mut Window) + 'static) {
    if let Some(active) = cx.active_window() {
        cx.defer(move |cx| {
            active.update(cx, |_, window, _| then(window)).ok();
        });
    }
}

/// Does not bring the window to the front: a window started in the background stays there.
pub fn open(socket: PathBuf, cx: &mut App) -> Result<Opened> {
    open_with(socket, yaar_navigator::NavigatorConfig::for_this_user(), cx)
}

/// [`open`], with the Navigator started on `navigator`. The app's quit waits up to
/// `STOP_DEADLINE` and a second more for the Navigator to stop its servers.
pub fn open_with(
    socket: PathBuf,
    navigator: yaar_navigator::NavigatorConfig,
    cx: &mut App,
) -> Result<Opened> {
    open_panes(
        socket,
        panes().iter().map(|pane| (pane.build)()).collect(),
        navigator,
        cx,
    )
}

pub fn open_panes(
    socket: PathBuf,
    panes: Vec<BuildPane>,
    navigator: yaar_navigator::NavigatorConfig,
    cx: &mut App,
) -> Result<Opened> {
    let opened = yaar_frame::open(socket, VERSION, panes, cx)?;
    join_navigator(&opened.models, navigator, cx);
    appearance::follow_system(&opened, cx);
    Ok(opened)
}

const STOP_WITHIN: Duration = Duration::from_secs(STOP_DEADLINE.as_secs() + 1);

struct Joined {
    navigator: Navigator,
    navigation: Entity<Navigation>,
}

#[derive(Default)]
struct Navigators {
    running: Vec<Joined>,
    stopped_at_quit: bool,
}

impl Global for Navigators {}

fn join_navigator(models: &Models, config: yaar_navigator::NavigatorConfig, cx: &mut App) {
    let (port, end) = yaar_navigator::port();
    let navigator = Navigator::start(config, end);
    models
        .navigation
        .update(cx, |navigation, cx| navigation.connect(port, cx));
    let navigators = cx.default_global::<Navigators>();
    navigators.running.push(Joined {
        navigator,
        navigation: models.navigation.clone(),
    });
    if !std::mem::replace(&mut navigators.stopped_at_quit, true) {
        // Drops every port while the windows stand, so each Navigator begins to stop. The future
        // waits after the windows are gone, so a pane's pending ask, which holds the port, goes
        // first.
        cx.on_app_quit(|cx| {
            let joined = std::mem::take(&mut cx.default_global::<Navigators>().running);
            for joined in &joined {
                drop_port(&joined.navigation, cx);
            }
            async move {
                // One deadline for all: the Navigators stop side by side.
                let deadline = Instant::now() + STOP_WITHIN;
                for joined in joined {
                    joined
                        .navigator
                        .wait(deadline.saturating_duration_since(Instant::now()));
                }
            }
        })
        .detach();
    }
}

fn drop_port(navigation: &Entity<Navigation>, cx: &mut App) {
    navigation.update(cx, |navigation, cx| navigation.disconnect(cx));
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Action, Menu, MenuItem, OwnedMenuItem};
    use yaar_frame::{Asks, Control, PROGRAM_CONTEXT, Place};

    use super::{Quit, controls, init, kept_keys, menus, panes};

    fn row(control: &Control) -> (&'static str, &'static str, Place) {
        (control.keystroke, (control.action)().name(), control.place)
    }

    #[test]
    fn the_table_holds_the_keys_of_the_frame_then_of_each_pane_then_the_window_programs_and_quit_on_secondary_q()
     {
        let mut expected: Vec<_> = yaar_frame::controls().iter().map(row).collect();
        expected.extend(yaar_frame::settings_screen::controls().iter().map(row));
        for pane in panes() {
            expected.extend((pane.controls)().iter().map(row));
        }
        expected.extend(super::own_controls().iter().map(row));

        let table = controls();
        assert_eq!(table.iter().map(row).collect::<Vec<_>>(), expected);

        let quit = table
            .iter()
            .find(|control| (control.action)().name() == Quit.name())
            .expect("the table has the plain quit");
        assert_eq!(row(quit), ("secondary-q", Quit.name(), Place::Anywhere));
        assert_eq!(quit.sends, None, "quitting the Window stops nothing");
        assert_eq!(quit.asks, Asks::Nothing);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_a_control_with_no_context_has_a_secondary_key_which_is_a_cmd_key_there() {
        let table = controls();
        for control in &table {
            if control.context().is_none() {
                assert!(
                    control.keystroke.starts_with("secondary-"),
                    "{} works with no context on the key {:?}",
                    control.title,
                    control.keystroke
                );
            }
        }
        assert!(table.iter().any(|control| control.context().is_none()));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_no_control_is_without_a_context() {
        for control in controls() {
            assert!(
                control.context().is_some(),
                "{} works with no context on the key {:?}",
                control.title,
                control.keystroke
            );
        }
    }

    #[test]
    fn no_key_belongs_to_two_controls_in_one_place() {
        // Of two rows with one key that both apply somewhere, one wins and nothing says so. A key
        // that works anywhere, a kept one included, applies inside every pane too.
        let everywhere = |place| matches!(place, Place::Anywhere | Place::Kept);
        let table = controls();
        for (at, control) in table.iter().enumerate() {
            for other in &table[at + 1..] {
                let in_one_place = control.place == other.place
                    || everywhere(control.place)
                    || everywhere(other.place);
                assert!(
                    control.keystroke != other.keystroke || !in_one_place,
                    "the key {:?} belongs to {} and to {}",
                    control.keystroke,
                    control.title,
                    other.title
                );
            }
        }
    }

    #[test]
    fn the_kept_keys_are_rows_of_the_table_each_with_a_key() {
        let table: Vec<_> = controls().iter().map(row).collect();
        let kept = kept_keys();
        assert!(!kept.is_empty());
        for control in &kept {
            assert!(table.contains(&row(control)), "{} is no row", control.title);
            assert!(!control.key().is_empty(), "{} has no key", control.title);
        }
        let mut keys: Vec<String> = kept.iter().map(Control::key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), kept.len(), "a key is kept twice");
    }

    #[test]
    fn the_keys_of_a_pane_that_shows_no_program_are_not_kept() {
        let kept: Vec<String> = kept_keys().iter().map(Control::key).collect();
        for control in yaar_review_pane::controls() {
            assert!(
                !kept.contains(&control.key()),
                "{} would never reach a program, and the review pane has none",
                control.title
            );
        }
    }

    #[test]
    fn a_key_that_works_anywhere_is_kept_where_it_is_a_cmd_key_and_not_elsewhere() {
        let kept: Vec<String> = kept_keys().iter().map(Control::key).collect();
        let anywhere: Vec<Control> = controls()
            .into_iter()
            .filter(|control| control.place == Place::Anywhere)
            .collect();
        assert!(!anywhere.is_empty());
        for control in anywhere {
            assert_eq!(
                kept.contains(&control.key()),
                cfg!(target_os = "macos"),
                "{} on {:?}",
                control.title,
                control.key()
            );
        }
    }

    #[test]
    fn the_keys_bound_in_a_terminal_and_the_kept_keys_of_the_frame_are_kept_on_every_system() {
        let kept: Vec<String> = kept_keys().iter().map(Control::key).collect();
        for control in controls() {
            if control.place == Place::Kept || control.place == Place::In(PROGRAM_CONTEXT) {
                assert!(kept.contains(&control.key()), "{}", control.title);
            }
        }
    }

    fn read(menu: &Menu) -> Vec<Option<String>> {
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => None,
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => panic!("the menu {} holds an item that is no action", menu.name),
            })
            .collect()
    }

    fn titled(title: &str) -> Option<String> {
        Some(title.to_string())
    }

    const MACOS_ONLY: [&str; 3] = ["Hide YAAR", "Hide Others", "Minimize"];

    #[test]
    fn the_menus_are_yaar_file_and_window_in_that_order() {
        let names: Vec<String> = menus().iter().map(|menu| menu.name.to_string()).collect();
        assert_eq!(names, ["YAAR", "File", "Window"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_the_menus_hold_every_item_of_the_doc_in_its_group() {
        let menus = menus();
        assert_eq!(
            read(&menus[0]),
            [
                titled("Settings…"),
                None,
                titled("Hide YAAR"),
                titled("Hide Others"),
                None,
                titled("Quit"),
                titled("Quit and Stop Agents"),
            ]
        );
        assert_eq!(
            read(&menus[1]),
            [
                titled("New Worktree with Agent…"),
                titled("Open Folder…"),
                None,
                titled("Close Window"),
            ]
        );
        assert_eq!(read(&menus[2]), [titled("Minimize")]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_the_menus_leave_out_hide_hide_others_and_minimize() {
        let menus = menus();
        assert_eq!(
            read(&menus[0]),
            [
                titled("Settings…"),
                None,
                titled("Quit"),
                titled("Quit and Stop Agents"),
            ]
        );
        assert_eq!(
            read(&menus[1]),
            [
                titled("New Worktree with Agent…"),
                titled("Open Folder…"),
                None,
                titled("Close Window"),
            ]
        );
        assert!(
            menus[2].items.is_empty(),
            "the Window menu holds Minimize alone"
        );
        for control in controls() {
            assert!(
                !MACOS_ONLY.contains(&control.title),
                "{} is a row of the table",
                control.title
            );
        }
    }

    #[test]
    fn each_item_runs_the_action_of_the_row_that_bears_its_title() {
        let table = controls();
        let mut seen = 0;
        for menu in menus() {
            for item in &menu.items {
                let MenuItem::Action { name, action, .. } = item else {
                    continue;
                };
                let rows: Vec<&Control> = table
                    .iter()
                    .filter(|control| control.title == name.as_ref())
                    .collect();
                assert_eq!(rows.len(), 1, "{name} is the title of {} rows", rows.len());
                assert!(
                    action.partial_eq(&*(rows[0].action)()),
                    "{name} in the {} menu runs {}, not the action of its row",
                    menu.name,
                    action.name()
                );
                seen += 1;
            }
        }
        // A title naming no row would be left out, and the test would pass over it.
        assert_eq!(seen, if cfg!(target_os = "macos") { 9 } else { 6 });
    }

    #[test]
    fn each_row_of_a_title_of_the_menus_is_the_action_that_title_names() {
        use yaar_frame::{CloseWindow, NewWorktree, OpenFolder, OpenSettings, QuitAndStopAgents};

        let mut expected: Vec<(&str, Box<dyn Action>)> = vec![
            ("Settings…", Box::new(OpenSettings)),
            ("Quit", Box::new(Quit)),
            ("Quit and Stop Agents", Box::new(QuitAndStopAgents)),
            ("New Worktree with Agent…", Box::new(NewWorktree)),
            ("Open Folder…", Box::new(OpenFolder)),
            ("Close Window", Box::new(CloseWindow)),
        ];
        if cfg!(target_os = "macos") {
            expected.extend([
                (
                    "Hide YAAR",
                    Box::new(yaar_frame::HideApp) as Box<dyn Action>,
                ),
                ("Hide Others", Box::new(yaar_frame::HideOtherApps)),
                ("Minimize", Box::new(yaar_frame::Minimize)),
            ]);
        }
        let table = controls();
        for (title, action) in &expected {
            let rows: Vec<&Control> = table
                .iter()
                .filter(|control| control.title == *title)
                .collect();
            assert_eq!(rows.len(), 1, "{title} is the title of {} rows", rows.len());
            let runs = (rows[0].action)();
            assert!(
                runs.partial_eq(&**action),
                "the row {title} runs {}, not {}",
                runs.name(),
                action.name()
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_the_rows_of_the_items_have_the_keys_of_every_mac_app() {
        let table = controls();
        let key_of = |title: &str| {
            table
                .iter()
                .find(|control| control.title == title)
                .unwrap_or_else(|| panic!("{title} has no row"))
                .key()
        };
        for (title, key) in [
            ("Settings…", "cmd-,"),
            ("Hide YAAR", "cmd-h"),
            ("Hide Others", "cmd-alt-h"),
            ("Quit", "cmd-q"),
            ("Quit and Stop Agents", "cmd-alt-q"),
            ("New Worktree with Agent…", "cmd-shift-n"),
            ("Open Folder…", "cmd-o"),
            ("Close Window", "cmd-w"),
            ("Minimize", "cmd-m"),
        ] {
            assert_eq!(key_of(title), key, "the key of {title}");
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_the_rows_of_the_items_have_the_ctrl_keys() {
        let table = controls();
        let key_of = |title: &str| {
            table
                .iter()
                .find(|control| control.title == title)
                .unwrap_or_else(|| panic!("{title} has no row"))
                .key()
        };
        for (title, key) in [
            ("Settings…", "ctrl-,"),
            ("Quit", "ctrl-q"),
            ("Quit and Stop Agents", "ctrl-alt-q"),
            ("New Worktree with Agent…", "ctrl-shift-n"),
            ("Open Folder…", "ctrl-o"),
            ("Close Window", "ctrl-w"),
        ] {
            assert_eq!(key_of(title), key, "the key of {title}");
        }
    }

    #[test]
    fn the_window_program_has_the_rows_close_window_and_on_macos_hide_hide_others_and_minimize() {
        let table = controls();
        let titles: Vec<&str> = table.iter().map(|control| control.title).collect();
        assert!(
            titles.contains(&"Close Window"),
            "Close Window is a row on every system"
        );
        for title in MACOS_ONLY {
            assert_eq!(
                titles.contains(&title),
                cfg!(target_os = "macos"),
                "{title} on this system"
            );
        }
        let own: Vec<&Control> = table
            .iter()
            .rev()
            .take_while(|control| {
                control.title == "Close Window" || MACOS_ONLY.contains(&control.title)
            })
            .collect();
        assert_eq!(own.len(), if cfg!(target_os = "macos") { 4 } else { 1 });
        for control in own {
            assert_eq!(control.place, Place::Anywhere, "{}", control.title);
            assert_eq!(control.sends, None, "{}", control.title);
            assert_eq!(control.asks, Asks::Nothing, "{}", control.title);
        }
    }

    #[gpui_kit::test]
    fn close_window_closes_the_window_that_has_the_focus_and_leaves_the_others(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{Context, IntoElement, Render, Window, div};

        struct Empty;
        impl Render for Empty {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
            }
        }

        cx.update(init);
        let first = cx.add_window(|_, _| Empty);
        let second = cx.add_window(|_, _| Empty);
        second
            .update(cx, |_, window, _| window.activate_window())
            .expect("the window is there");
        assert_eq!(cx.update(|cx| cx.windows().len()), 2);
        assert_eq!(
            cx.update(|cx| cx.active_window())
                .map(|window| window.window_id()),
            Some(second.window_id())
        );

        let close = controls()
            .into_iter()
            .find(|control| control.title == "Close Window")
            .expect("the table has the row Close Window");
        cx.update(|cx| cx.dispatch_action(&*(close.action)()));
        cx.run_until_parked();
        let left: Vec<_> = cx.update(|cx| cx.windows().iter().map(|w| w.window_id()).collect());
        assert_eq!(
            left,
            [first.window_id()],
            "the active window is the one closed"
        );
    }

    #[gpui_kit::test]
    fn init_hands_the_platform_the_menus(cx: &mut gpui_kit::TestAppContext) {
        let shown = cx.update(|cx| {
            init(cx);
            cx.get_menus().expect("the platform keeps the menus")
        });
        let expected = menus();
        assert!(!expected.is_empty());
        assert_eq!(shown.len(), expected.len());
        for (shown, expected) in shown.iter().zip(&expected) {
            assert_eq!(shown.name, expected.name);
            let names: Vec<Option<String>> = shown
                .items
                .iter()
                .map(|item| match item {
                    OwnedMenuItem::Separator => None,
                    OwnedMenuItem::Action { name, .. } => Some(name.clone()),
                    _ => panic!("an item that is no action"),
                })
                .collect();
            assert_eq!(names, read(expected));
        }
    }
}
