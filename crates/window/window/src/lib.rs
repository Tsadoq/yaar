//! The Window program: the process that draws.
//!
//! It registers the panes in the Frame and binds the keys; it holds nothing of its own to
//! show. The program `yaar-review` is this library behind a command line.
//!
//! Surface:
//! - [`panes`], the one list of the panes of the Window. A pane is added by one line
//!   there, and has its keys bound and its view built from that line;
//! - [`init`], which prepares the app before a window opens;
//! - [`open`], which opens the window with every pane, linked to the Keeper, and starts the
//!   Navigator with what this user's environment gives it; the quit of the app is the one
//!   end of every Navigator that runs;
//! - [`open_with`] and [`open_panes`], the entries beneath `open` that the component tests of
//!   the composed Window take: the same window with a Navigator's configuration of the
//!   caller's, and with panes of the caller's too;
//! - [`controls`], every action of the Window that has a key: the Frame's, [`Quit`] among
//!   them, the settings screen's, each pane's, and the Window program's own (Close Window,
//!   and on macOS Hide, Hide Others and Minimize). Every message of the Link that a user
//!   starts is the end of one of them;
//! - [`kept_keys`], those of them whose key works while a terminal has the focus: the
//!   documented set of keys that never reach the program in a terminal;
//! - [`menus`], the menus of the app on macOS, made from [`controls`]: GPUI shows each
//!   item's key from the binding of its action.

// The system's light or dark setting, told to the theme's model.
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

/// Quits the Window, the plain way: the Keeper and what it holds stay. One of the Frame's
/// two quits, beside `yaar_frame::QuitAndStopAgents`; the Window program says what it does.
pub use yaar_frame::Quit;

/// The version of the app, as shown to the user and told to the Keeper.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The panes of the Window, in the order they are laid out. Both [`controls`] and
/// [`open`] read this list and nothing else about panes.
pub fn panes() -> Vec<PaneEntry> {
    vec![yaar_review_pane::entry(), yaar_terminal_pane::entry()]
}

/// The documented set of keys that never reach a program in a terminal: every control of
/// [`controls`] whose key works while a terminal has the focus. On macOS these are the
/// Cmd keys of the app, the kept keys of the Frame and the terminal's own copy, paste and
/// scrolling of the history; on Linux the kept keys and the terminal's own alone, since
/// there the app's keys are Ctrl keys, and every other Ctrl key belongs to the program.
pub fn kept_keys() -> Vec<Control> {
    controls()
        .into_iter()
        .filter(|control| match control.place {
            Place::Kept => true,
            Place::In(context) => context == PROGRAM_CONTEXT,
            // Bound for the whole app only where it is a Cmd key.
            Place::Anywhere => control.context().is_none(),
        })
        .collect()
}

/// The menus of the app, which [`init`] hands GPUI (`App::set_menus`): on macOS the menu bar
/// shows them, with each item's key; GPUI's Linux platforms show none, and their keys work as
/// the table binds them.
///
/// | Menu | Items, in order (a line between groups) |
/// | --- | --- |
/// | YAAR | Settings…; Hide YAAR, Hide Others; Quit, Quit and Stop Agents |
/// | File | New Worktree with Agent…, Open Folder…; Close Window |
/// | Window | Minimize |
///
/// Each item runs the action of the row of [`controls`] whose title it bears, so that a key
/// and an item are one control. Hide YAAR, Hide Others and Minimize are rows on macOS only;
/// off macOS the menus leave them out.
pub fn menus() -> Vec<Menu> {
    let table = controls();
    // Titles for each group of each menu; a title that has no row on this system, as Hide
    // has none off macOS, is left out, and a group left empty with it.
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

/// Every action of the Window that has a key: the Frame's, the two quits among them, the
/// settings screen's, each pane's, and last the Window program's own: Close Window on every
/// system and, on macOS, Hide YAAR, Hide Others and Minimize.
pub fn controls() -> Vec<Control> {
    let mut controls = yaar_frame::controls();
    // The settings screen is the Frame's, with keys of its own context, as a pane has.
    controls.extend(yaar_frame::settings_screen::controls());
    for pane in panes() {
        controls.extend((pane.controls)());
    }
    controls.extend(own_controls());
    controls
}

/// The rows of the Window program itself, which the Frame and the panes know nothing of.
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
    // Hiding and minimising are the system's on macOS; elsewhere the window manager has
    // its own keys for them.
    if cfg!(target_os = "macos") {
        rows.extend([
            window("Hide YAAR", || Box::new(HideApp), "secondary-h"),
            window("Hide Others", || Box::new(HideOtherApps), "secondary-alt-h"),
            window("Minimize", || Box::new(Minimize), "secondary-m"),
        ]);
    }
    rows
}

/// Prepares the app: the toolkit, the theme of the system, the keys of [`controls`], the
/// toolkit's own keys given back to a program in a terminal, the menus of [`menus`], what
/// quitting means, and what the actions of the Window program do.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
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

/// The handlers of [`own_controls`]. A window's action reaches the window that has the
/// focus: the app's handler asks the platform which that is, since a menu item runs the
/// action with no element to start from.
fn handle_window_actions(cx: &mut App) {
    cx.on_action(|_: &CloseWindow, cx| {
        // The app quits the plain way once its last window is closed (see `init`).
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

/// Runs `then` on the window that has the focus; nothing when the app has none.
///
/// An action handler of the app runs while the window it came from is being updated, and
/// that window cannot be updated again until the update ends, so the work is deferred to
/// the end of the current effects.
fn with_active_window(cx: &mut App, then: impl FnOnce(&mut Window) + 'static) {
    if let Some(active) = cx.active_window() {
        cx.defer(move |cx| {
            // A window that closed meanwhile is nothing to act on.
            active.update(cx, |_, window, _| then(window)).ok();
        });
    }
}

/// Opens the window with every pane and links it to the Keeper on `socket`, starting the
/// Keeper when none answers.
///
/// The system brings an app it launches to the front. The Window does not push itself
/// there: a window started in the background stays in the background.
pub fn open(socket: PathBuf, cx: &mut App) -> Result<Opened> {
    open_with(socket, yaar_navigator::NavigatorConfig::for_this_user(), cx)
}

/// [`open`], with the Navigator started on `navigator`: the folders, the registry and the
/// environment of its servers.
///
/// The Navigator is started on a new port and joined to the window's model of code
/// intelligence. The quit of the app is the one end of a Navigator: the
/// port is let go ([`yaar_frame::Navigation::disconnect`]) and the Window waits up to
/// [`yaar_navigator::STOP_DEADLINE`] and a second more for the Navigator to stop its servers,
/// so that no server outlives the Window, whatever asks are on their way. A second window
/// gets a Navigator of its own.
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

/// [`open_with`] for panes the caller builds: the Frame opened on `panes` and the Navigator
/// joined to it. The caller may wrap the builders of [`panes`] to keep the views it reads.
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

/// How long the Window waits for a Navigator to stop its servers: the time a server is
/// given to end, and a second more.
const STOP_WITHIN: Duration = Duration::from_secs(STOP_DEADLINE.as_secs() + 1);

/// A Navigator the Window started, and the model it is joined to.
struct Joined {
    navigator: Navigator,
    navigation: Entity<Navigation>,
}

/// The Navigators of the app's windows, one each, kept for as long as they run.
#[derive(Default)]
struct Navigators {
    running: Vec<Joined>,
    /// Whether the app's quit stops them: said once, by the first window.
    stopped_at_quit: bool,
}

impl Global for Navigators {}

/// Starts a Navigator on `config`, joins its port to `models.navigation` and keeps its
/// handle in a global of the app, to be stopped at the quit of the app. What the
/// model tells the Navigator by itself, and what it keeps of what it is told, is the Frame's.
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
        // The handler's own part runs while the windows stand: it drops every port, so
        // that each Navigator begins to stop its servers. The waiting is the future's, run
        // after the windows are gone, so that a pane's ask still on its way, which holds
        // the port too, is dropped with its pane first. It blocks the thread: the quit
        // goes on only once no server runs, or at the deadline.
        cx.on_app_quit(|cx| {
            let joined = std::mem::take(&mut cx.default_global::<Navigators>().running);
            for joined in &joined {
                drop_port(&joined.navigation, cx);
            }
            async move {
                // One deadline for all: the Navigators stop side by side, and the quit
                // never waits longer than a single one may take.
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

/// Lets go of the Frame's end of a Navigator's port: the model closes the channel of asks
/// for every sender made from it, those a pane's ask on its way and an export the Keeper has
/// not yet answered hold among them, so that the Navigator, with no sender left, stops its
/// servers and ends.
fn drop_port(navigation: &Entity<Navigation>, cx: &mut App) {
    navigation.update(cx, |navigation, cx| navigation.disconnect(cx));
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Action, Menu, MenuItem, OwnedMenuItem};
    use yaar_frame::{Asks, Control, PROGRAM_CONTEXT, Place};

    use super::{Quit, controls, init, kept_keys, menus, panes};

    /// What tells one row of the table from another.
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

        // The plain quit, written out here so that a change of its key in the table is a
        // change in two places.
        let quit = table
            .iter()
            .find(|control| (control.action)().name() == Quit.name())
            .expect("the table has the plain quit");
        assert_eq!(row(quit), ("secondary-q", Quit.name(), Place::Anywhere));
        assert_eq!(quit.sends, None, "quitting the Window stops nothing");
        assert_eq!(quit.asks, Asks::Nothing);
    }

    // The next two tests are one rule on two kinds of system: only a Cmd key is bound with
    // no context. Any other key bound so would be taken from the program in a terminal,
    // and from every field a user types into.

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
        // The rule is about the keys of the Frame and of the Window program, and they
        // are there to be held to it.
        assert!(table.iter().any(|control| control.context().is_none()));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_no_control_is_without_a_context() {
        // The keyboard has no Cmd key, and `secondary` is Ctrl.
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
        // The parts of the table come from crates that do not know each other. Of two
        // rows with one key that both apply somewhere, one wins there and nothing says
        // so. Both apply where their places are the same, and wherever one of them
        // works everywhere, as a key of the Frame does, a kept one included: that one
        // applies inside every pane too. A key bound in a terminal applies in the
        // terminal pane, which no other pane's key shares.
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

    /// A menu as a user reads it: the title of each item, a `None` for a line between
    /// groups.
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

    /// The actions of the table, which no system but macOS binds.
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
        // The rows are gone as well: no key, and so no item.
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
        // Every title of the doc's table is an item: a title that names no row would be
        // left out, and the test would pass over it.
        assert_eq!(seen, if cfg!(target_os = "macos") { 9 } else { 6 });
    }

    #[test]
    fn each_row_of_a_title_of_the_menus_is_the_action_that_title_names() {
        use yaar_frame::{CloseWindow, NewWorktree, OpenFolder, OpenSettings, QuitAndStopAgents};

        // The test above ties an item to its row; this one ties the row to what the title
        // says, so that a key and an item that run the wrong action are seen.
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
        // After the panes' rows, as the table's doc says; none of them sends or asks.
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

        // The action of the row of the table, which is what its key and its menu item run.
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
