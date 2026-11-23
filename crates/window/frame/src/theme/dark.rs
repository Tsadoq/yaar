//! The dark theme the app is built with. Its values: see the module doc of `theme`.

use super::{Appearance, Colors, TerminalColors, ThemeDefinition, hex, hexa};

/// The dark theme, named [`super::DARK`].
pub fn dark() -> ThemeDefinition {
    let colors = Colors {
        background: hex(0x0a0a0a),
        foreground: hex(0xfafafa),
        border: hex(0x262626),
        input: hex(0x2f2f2f),
        ring: hex(0x737373),
        caret: hex(0xfafafa),
        selection: hexa(0x1d4ed84d),
        primary: hex(0xfafafa),
        primary_foreground: hex(0x171717),
        secondary: hex(0x262626),
        secondary_foreground: hex(0xfafafa),
        muted: hex(0x262626),
        muted_foreground: hex(0xa3a3a3),
        accent: hex(0x262626),
        accent_foreground: hex(0xfafafa),
        list_hover: hexa(0x26262699),
        success: hex(0x4ade80),
        warning: hex(0xfacc15),
        // The toolkit's #ca8a04 reads 1.92 to 1 on `warning`. It lies darker than that light
        // yellow, so the steps away from the yellow's lightness are darker ones, and the first
        // that reads 4.5 to 1 is this.
        warning_foreground: hex(0x664602),
        danger: hex(0xf87171),
        info: hex(0x22d3ee),
        sidebar: hex(0x0a0a0a),
        sidebar_foreground: hex(0xf5f5f5),
        sidebar_border: hex(0x262626),
        sidebar_accent: hex(0x262626),
        sidebar_accent_foreground: hex(0xf5f5f5),
        red: hex(0xf87171),
        red_light: hex(0xfca5a5),
        green: hex(0x4ade80),
        green_light: hex(0x86efac),
        yellow: hex(0xfacc15),
        yellow_light: hex(0xfde047),
        blue: hex(0x60a5fa),
        blue_light: hex(0x93c5fd),
        magenta: hex(0xc084fc),
        magenta_light: hex(0xd8b4fe),
        cyan: hex(0x22d3ee),
        cyan_light: hex(0x67e8f9),
    };
    ThemeDefinition {
        name: super::DARK.to_owned(),
        appearance: Appearance::Dark,
        terminal: TerminalColors {
            foreground: colors.foreground,
            background: colors.background,
            cursor: colors.foreground,
            selection: colors.selection,
            ansi: [
                hex(0x000000),
                colors.red,
                colors.green,
                colors.yellow,
                colors.blue,
                colors.magenta,
                colors.cyan,
                hex(0xe5e5e5),
                hex(0x7f7f7f),
                colors.red_light,
                colors.green_light,
                colors.yellow_light,
                colors.blue_light,
                colors.magenta_light,
                colors.cyan_light,
                hex(0xffffff),
            ],
        },
        colors,
    }
}
