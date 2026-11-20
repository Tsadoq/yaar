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
//!   Navigator with what this user's environment gives it; [`open_with`] does the same with
//!   a Navigator's configuration of the caller's;
//! - [`controls`], every action of the Window that has a key: the Frame's, each pane's,
//!   and [`Quit`]. Every message of the Link that a user starts is the end of one of them;
//! - [`kept_keys`], those of them whose key works while a terminal has the focus: the
//!   documented set of keys that never reach the program in a terminal.

use std::path::PathBuf;

use gpui_kit::component::theme::Theme;
use gpui_kit::*;
use yaar_frame::{Control, Opened, PROGRAM_CONTEXT, PaneEntry, Place};

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

/// Every action of the Window that has a key: the Frame's, the two quits among them, then
/// each pane's.
pub fn controls() -> Vec<Control> {
    let mut controls = yaar_frame::controls();
    for pane in panes() {
        controls.extend((pane.controls)());
    }
    controls
}

/// Prepares the app: the toolkit, the theme of the system, the keys of [`controls`], the
/// toolkit's own keys given back to a program in a terminal, and what quitting means.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    // After the toolkit, whose bindings it has to outrank.
    yaar_frame::release_program_keys(cx);
    Theme::sync_system_appearance(None, cx);
    yaar_frame::bind(&controls(), cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
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
/// environment of its servers. A test of the composed Window hands one of its own, with the
/// stand-in language server in its registry and its folders in the test's own.
///
/// The Navigator is started on a new port (`yaar_navigator::port`), its end handed to
/// `yaar_navigator::Navigator::start` and the Frame's end to the Frame's model
/// `Models::navigation` (`Navigation::connect`). The Navigator's handle is kept in a global
/// of the app; when the app quits, the model's port is dropped and the Window waits up to
/// `yaar_navigator::STOP_DEADLINE` and a second more for the Navigator to stop its servers,
/// so that no server outlives the Window (ADR-015 point 10). A second window gets a
/// Navigator of its own.
pub fn open_with(
    socket: PathBuf,
    navigator: yaar_navigator::NavigatorConfig,
    cx: &mut App,
) -> Result<Opened> {
    // The Window program's package starts the Navigator here.
    let _ = navigator;
    let panes = panes().iter().map(|pane| (pane.build)()).collect();
    yaar_frame::open(socket, VERSION, panes, cx)
}

#[cfg(test)]
mod tests {
    use gpui_kit::Action;
    use yaar_frame::{Asks, Control, PROGRAM_CONTEXT, Place};

    use super::{Quit, controls, kept_keys, panes};

    /// What tells one row of the table from another.
    fn row(control: &Control) -> (&'static str, &'static str, Place) {
        (control.keystroke, (control.action)().name(), control.place)
    }

    #[test]
    fn the_table_holds_the_keys_of_the_frame_then_of_each_pane_and_quit_on_secondary_q() {
        let mut expected: Vec<_> = yaar_frame::controls().iter().map(row).collect();
        for pane in panes() {
            expected.extend((pane.controls)().iter().map(row));
        }

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
}
