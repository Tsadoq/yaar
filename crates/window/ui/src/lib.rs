use std::borrow::Cow;

use gpui_kit::App;

pub mod colour;
pub mod fonts;
pub mod metrics;
pub mod themes;
pub mod tokens;
pub mod typography;

/// Registers the built-in fonts. The theme is applied by whoever owns the appearance.
pub fn init(cx: &mut App) {
    let built_in = fonts::BUILT_IN.iter().map(|bytes| Cow::Borrowed(*bytes));
    if let Err(error) = fonts::register(cx, built_in.collect()) {
        eprintln!("yaar-review: {error}");
    }
}
