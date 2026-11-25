use std::time::Duration;

use gpui_kit::{BoxShadow, CursorStyle, FontFeatures, Pixels, Styled, hsla, point, px};

pub const RADIUS_WINDOW: Pixels = px(12.);
pub const RADIUS_CARD: Pixels = px(10.);
pub const RADIUS_CONTROL: Pixels = px(6.);
pub const RADIUS_INLINE: Pixels = px(4.);
pub const RADIUS_SMALL: Pixels = px(3.);

pub const SPACE_1: Pixels = px(2.);
pub const SPACE_2: Pixels = px(4.);
pub const SPACE_3: Pixels = px(6.);
pub const SPACE_4: Pixels = px(8.);
pub const SPACE_5: Pixels = px(10.);
pub const SPACE_6: Pixels = px(12.);
pub const SPACE_7: Pixels = px(14.);
pub const SPACE_8: Pixels = px(16.);
pub const GUTTER: Pixels = px(8.);

pub const FADE_IN: Duration = Duration::from_millis(140);
pub const PULSE: Duration = Duration::from_millis(1_000);
pub const PULSE_SIDEBAR: Duration = Duration::from_millis(1_200);
pub const FLASH: Duration = Duration::from_millis(1_600);

fn black_shadow(offset_y: f32, blur: f32, alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0., 0., 0., alpha),
        offset: point(px(0.), px(offset_y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Under a popover, a menu or a toast.
pub fn shadow_float() -> Vec<BoxShadow> {
    black_shadow(6., 20., 0.5)
}

/// Under a card that floats over the window.
pub fn shadow_card() -> Vec<BoxShadow> {
    black_shadow(10., 30., 0.55)
}

/// Equal-width digits. A function, not a constant: the feature list is a shared allocation.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

/// Text-beam cursor for painted terminal surfaces.
pub fn ibeam<T: Styled>(element: T) -> T {
    element.cursor(CursorStyle::IBeam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_radii_step_down_from_window_to_inline() {
        assert!(RADIUS_WINDOW > RADIUS_CARD);
        assert!(RADIUS_CARD > RADIUS_CONTROL);
        assert!(RADIUS_CONTROL > RADIUS_INLINE);
        assert!(RADIUS_INLINE > RADIUS_SMALL);
    }
}
