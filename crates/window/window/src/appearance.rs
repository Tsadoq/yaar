//! The system's light or dark setting, told to the window's model of the theme
//! (`yaar_frame::Looks`, NFR-018).
//!
//! [`follow_system`] runs once a window is opened, before it is first drawn:
//!
//! 1. it tells `Looks::system_is` the window's appearance at once (`Window::appearance`,
//!    turned into `yaar_frame::theme::Appearance`), so that the theme of the system is
//!    applied from the first frame;
//! 2. it observes the window's appearance (`Window::observe_window_appearance`, the
//!    subscription detached for the window's life) and tells `Looks::system_is` each change;
//! 3. it observes `Looks`: when the user's choice goes back to the system
//!    (`Looks::chosen` becomes `ThemeChoice::System` from a named theme), the window's
//!    forced appearance is let go by `Looks` itself, and the window's appearance is read
//!    again and told, since the observation of step 2 saw only the forced one meanwhile.
//!
//! On GPUI's test platform the window's appearance is light and never changes: the
//! composed Window's test (WIN-11, proposed) reads that `Looks::system` is that light
//! appearance right after the opening. The changes of step 2 are seen only on a real
//! screen, by the checklist of the platforms' conventions.

use gpui_kit::*;
use yaar_frame::Opened;

/// Tells the theme's model of `opened` the system's setting now and at each change, as the
/// module doc says. Until it is built it tells nothing.
pub(crate) fn follow_system(opened: &Opened, cx: &mut App) {
    let _ = (opened, cx);
}
