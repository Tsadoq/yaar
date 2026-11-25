//! The system's light or dark setting, told to `Looks`. When a forced theme is let go, the
//! system's appearance is read again from the application, not the window: the window keeps
//! the forced value until the platform tells it again.

use gpui_kit::*;
use yaar_frame::{Looks, Opened};
use yaar_link::ThemeChoice;
use yaar_ui::themes::Appearance;

pub(crate) fn follow_system(opened: &Opened, cx: &mut App) {
    let looks = opened.models.looks.clone();
    let told = opened.window.update(cx, |_, window, cx| {
        tell(&looks, window.appearance(), cx);
        let looks = looks.clone();
        window
            .observe_window_appearance(move |window, cx| tell(&looks, window.appearance(), cx))
            .detach();
    });
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

fn tell(looks: &Entity<Looks>, appearance: WindowAppearance, cx: &mut App) {
    looks.update(cx, |looks, cx| {
        looks.system_is(light_or_dark(appearance), cx)
    });
}

fn light_or_dark(appearance: WindowAppearance) -> Appearance {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

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
