//! The Window program: the process that draws.
//!
//! It registers the panes in the Frame and binds the keys; it holds nothing of its own to
//! show. The program `yaar-review` is this library behind a command line.
//!
//! Surface:
//! - [`panes`], the one list of the panes of the Window. A pane is added by one line
//!   there, and has its keys bound and its view built from that line;
//! - [`init`], which prepares the app before a window opens;
//! - [`open`], which opens the window with every pane, linked to the Keeper;
//! - [`controls`], every action of the Window that has a key: the Frame's, each pane's,
//!   and [`Quit`]. Every message of the Link that a user starts is the end of one of them.

use std::path::PathBuf;

use gpui_kit::component::theme::Theme;
use gpui_kit::*;
use yaar_frame::{Asks, Control, Opened, PaneEntry, Place};

/// The version of the app, as shown to the user and told to the Keeper.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

actions!(
    yaar,
    [
        /// Quits the Window. The Keeper and what it holds stay.
        Quit
    ]
);

/// The panes of the Window, in the order they are laid out. Both [`controls`] and
/// [`open`] read this list and nothing else about panes.
pub fn panes() -> Vec<PaneEntry> {
    vec![yaar_review_pane::entry()]
}

/// Every action of the Window that has a key.
pub fn controls() -> Vec<Control> {
    let mut controls = yaar_frame::controls();
    for pane in panes() {
        controls.extend((pane.controls)());
    }
    controls.push(Control {
        title: "Quit",
        action: || Box::new(Quit),
        keystroke: "secondary-q",
        place: Place::Anywhere,
        sends: None,
        asks: Asks::Nothing,
    });
    controls
}

/// Prepares the app: the toolkit, the theme of the system, the keys of [`controls`], and
/// what quitting means.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
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
    let panes = panes().iter().map(|pane| (pane.build)()).collect();
    yaar_frame::open(socket, VERSION, panes, cx)
}

#[cfg(test)]
mod tests {
    use gpui_kit::Action;
    use yaar_frame::{Asks, Control, Place};

    use super::{Quit, controls, panes};

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
        // The one key of the Window program's own, written out here so that a change of
        // it in the table is a change in two places.
        expected.push(("secondary-q", Quit.name(), Place::Anywhere));

        let table = controls();
        assert_eq!(table.iter().map(row).collect::<Vec<_>>(), expected);

        let quit = table.last().expect("the table has rows");
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
        // works anywhere: that one applies inside every pane too.
        let table = controls();
        for (at, control) in table.iter().enumerate() {
            for other in &table[at + 1..] {
                let in_one_place = control.place == other.place
                    || control.place == Place::Anywhere
                    || other.place == Place::Anywhere;
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
}
