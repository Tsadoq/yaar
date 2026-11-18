//! The Window program: the process that draws.
//!
//! It registers the panes in the Frame and binds the keys; it holds nothing of its own to
//! show. The program `yaar-review` is this library behind a command line.
//!
//! Surface:
//! - [`init`], which prepares the app before a window opens;
//! - [`open`], which opens the window with every pane, linked to the Keeper;
//! - [`controls`], every action of the Window that has a key: the Frame's, each pane's,
//!   and [`Quit`]. Every request of the Link that a user starts is the end of one of them.

use std::path::PathBuf;

use gpui_kit::component::theme::Theme;
use gpui_kit::*;
use yaar_frame::{Asks, Control, Opened};

/// The version of the app, as shown to the user and told to the Keeper.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

actions!(
    yaar,
    [
        /// Quits the Window. The Keeper and what it holds stay.
        Quit
    ]
);

/// Every action of the Window that has a key.
pub fn controls() -> Vec<Control> {
    let mut controls = yaar_frame::controls();
    controls.extend(yaar_review_pane::controls());
    controls.push(Control {
        title: "Quit",
        action: || Box::new(Quit),
        keystroke: "secondary-q",
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
    yaar_frame::open(socket, VERSION, vec![yaar_review_pane::pane()], cx)
}
