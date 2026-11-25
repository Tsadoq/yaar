use std::borrow::Cow;
use std::fmt;

use gpui_kit::App;

pub const UI: &str = "Inter";

pub const CODE: &str = "JetBrains Mono";

/// Inter regular, medium and semibold, then JetBrains Mono regular and medium.
pub const BUILT_IN: [&[u8]; 5] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-Medium.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Medium.ttf"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontError(pub String);

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot register the fonts: {}", self.0)
    }
}

impl std::error::Error for FontError {}

/// The first four bytes of a TrueType, OpenType or collection file.
fn is_font_file(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf")
    )
}

/// Refuses an item that does not open with a font file signature before the text system sees any
/// item. Past that check the platform decides, and may refuse a corrupt font or skip it.
pub fn register(cx: &App, fonts: Vec<Cow<'static, [u8]>>) -> Result<(), FontError> {
    if let Some(at) = fonts.iter().position(|bytes| !is_font_file(bytes)) {
        return Err(FontError(format!("item {at} is not a font file")));
    }
    cx.text_system()
        .add_fonts(fonts)
        .map_err(|error| FontError(error.to_string()))
}
