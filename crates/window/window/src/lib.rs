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
use yaar_frame::{Asks, Control, Opened, PaneEntry};

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
        keystroke: "cmd-q",
        context: None,
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
    use yaar_frame::{Asks, Control};

    use super::{Quit, controls, panes};

    /// What tells one row of the table from another.
    fn row(control: &Control) -> (&'static str, &'static str, Option<&'static str>) {
        (
            control.keystroke,
            (control.action)().name(),
            control.context,
        )
    }

    #[test]
    fn the_table_holds_the_keys_of_the_frame_then_of_each_pane_and_quit_on_cmd_q() {
        let mut expected: Vec<_> = yaar_frame::controls().iter().map(row).collect();
        for pane in panes() {
            expected.extend((pane.controls)().iter().map(row));
        }
        // The one key of the Window program's own, written out here so that a change of
        // it in the table is a change in two places.
        expected.push(("cmd-q", Quit.name(), None));

        let table = controls();
        assert_eq!(table.iter().map(row).collect::<Vec<_>>(), expected);

        let quit = table.last().expect("the table has rows");
        assert_eq!(quit.sends, None, "quitting the Window stops nothing");
        assert_eq!(quit.asks, Asks::Nothing);
    }

    #[test]
    fn a_key_that_works_anywhere_is_a_cmd_key() {
        // Any other key bound with no context would be taken from the program in a
        // terminal, and from every field a user types into.
        for control in controls() {
            if control.context.is_none() {
                assert!(
                    control.keystroke.starts_with("cmd-"),
                    "{} works anywhere on the key {:?}",
                    control.title,
                    control.keystroke
                );
            }
        }
    }

    #[test]
    fn no_key_belongs_to_two_controls_in_one_place() {
        // The parts of the table come from crates that do not know each other. Of two
        // rows with one key that both apply somewhere, one wins there and nothing says
        // so. Both apply where their contexts are the same, and wherever one of them
        // has none.
        let table = controls();
        for (at, control) in table.iter().enumerate() {
            for other in &table[at + 1..] {
                let in_one_place = control.context == other.context
                    || control.context.is_none()
                    || other.context.is_none();
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
