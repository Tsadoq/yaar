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
//! 3. it observes `Looks`: when a forced appearance is let go, which is when the user's
//!    choice goes from a theme the Window has to the system's, and also when it goes to a
//!    name no theme has (`Looks` lets the window's forced appearance go in both), the
//!    appearance is read again and told, since the observation of step 2 saw only the
//!    forced one meanwhile. It is read from the application (`App::window_appearance`, what
//!    the system reports for the application's windows) and not from the window: the window
//!    keeps the value it was last told, which until the platform tells it again is the
//!    forced one. A choice that moves from one theme to another, or stays the system's, lets
//!    nothing go and tells nothing.
//!
//! On GPUI's test platform the window's appearance is light and never changes: the
//! composed Window's test (WIN-11) reads that `Looks::system` is that light appearance right
//! after the opening, and plays the forced one noted by telling the model a dark appearance
//! itself, to show that step 3 reads the platform's again. The changes of steps 2 and 3 are
//! seen on a real screen only, by the checklist of the platforms' conventions.

use gpui_kit::*;
use yaar_frame::theme::Appearance;
use yaar_frame::{Looks, Opened};
use yaar_link::ThemeChoice;

/// Tells the theme's model of `opened` the system's setting now and at each change, as the
/// module doc says.
pub(crate) fn follow_system(opened: &Opened, cx: &mut App) {
    let looks = opened.models.looks.clone();
    let told = opened.window.update(cx, |_, window, cx| {
        tell(&looks, window.appearance(), cx);
        let looks = looks.clone();
        window
            .observe_window_appearance(move |window, cx| tell(&looks, window.appearance(), cx))
            .detach();
    });
    // A window that is gone before it is told has nothing to follow.
    if told.is_err() {
        return;
    }

    let mut forced = is_forced(&looks, cx);
    cx.observe(&looks, move |looks, cx| {
        let now = is_forced(&looks, cx);
        // Set before telling: telling `Looks` notifies it, and this runs again.
        let was = std::mem::replace(&mut forced, now);
        if was && !now {
            tell(&looks, cx.window_appearance(), cx);
        }
    })
    .detach();
}

/// Tells `looks` that the system's setting is the one `appearance` stands for.
fn tell(looks: &Entity<Looks>, appearance: WindowAppearance, cx: &mut App) {
    looks.update(cx, |looks, cx| {
        looks.system_is(light_or_dark(appearance), cx)
    });
}

/// The setting of the system a window appearance stands for: the vibrant ones are the light
/// and the dark ones with the window's translucency.
fn light_or_dark(appearance: WindowAppearance) -> Appearance {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

/// Whether the user's choice is a theme the Window has, so that the window's own appearance
/// is forced to that theme's (`Looks` does it).
fn is_forced(looks: &Entity<Looks>, cx: &App) -> bool {
    let looks = looks.read(cx);
    matches!(looks.chosen(cx), ThemeChoice::Named(name) if looks.themes().named(&name).is_some())
}

#[cfg(test)]
mod tests {
    use gpui_kit::WindowAppearance;

    use super::{Appearance, light_or_dark};

    #[test]
    fn the_vibrant_appearances_are_the_light_and_the_dark_ones() {
        assert_eq!(light_or_dark(WindowAppearance::Light), Appearance::Light);
        assert_eq!(
            light_or_dark(WindowAppearance::VibrantLight),
            Appearance::Light
        );
        assert_eq!(light_or_dark(WindowAppearance::Dark), Appearance::Dark);
        assert_eq!(
            light_or_dark(WindowAppearance::VibrantDark),
            Appearance::Dark
        );
    }
}
