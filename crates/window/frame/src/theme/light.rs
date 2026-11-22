//! The light theme the app is built with. Its values: see the module doc of `theme`.

use super::{Appearance, Colors, TerminalColors, ThemeDefinition, hex, hexa};

/// The light theme, named [`super::LIGHT`].
pub fn light() -> ThemeDefinition {
    let colors = Colors {
        background: hex(0xffffff),
        foreground: hex(0x0a0a0a),
        border: hex(0xe5e5e5),
        input: hex(0xe5e5e5),
        ring: hex(0xa3a3a3),
        caret: hex(0x0a0a0a),
        selection: hexa(0x55a0fc4d),
        primary: hex(0x171717),
        primary_foreground: hex(0xfafafa),
        secondary: hex(0xe5e5e5),
        secondary_foreground: hex(0x171717),
        muted: hex(0xf5f5f5),
        muted_foreground: hex(0x737373),
        accent: hex(0xf5f5f5),
        accent_foreground: hex(0x171717),
        list_hover: hex(0xf5f5f5),
        success: hex(0x22c55e),
        warning: hex(0xeab308),
        // The toolkit's #fafafa reads 1.84 to 1 on `warning`: darkened until 4.5 to 1.
        warning_foreground: hex(0x474747),
        // The toolkit's #ef4444 reads 3.76 to 1 on `background`: darkened until 4.5 to 1.
        danger: hex(0xeb1515),
        info: hex(0x06b6d4),
        sidebar: hex(0xfafafa),
        sidebar_foreground: hex(0x171717),
        sidebar_border: hex(0xe5e5e5),
        sidebar_accent: hex(0xe5e5e5),
        sidebar_accent_foreground: hex(0x171717),
        red: hex(0xdc2626),
        red_light: hex(0xf87171),
        green: hex(0x16a34a),
        green_light: hex(0x4ade80),
        yellow: hex(0xca8a04),
        yellow_light: hex(0xfacc15),
        blue: hex(0x2563eb),
        blue_light: hex(0x60a5fa),
        magenta: hex(0x9333ea),
        magenta_light: hex(0xc084fc),
        cyan: hex(0x0891b2),
        cyan_light: hex(0x22d3ee),
    };
    ThemeDefinition {
        name: super::LIGHT.to_owned(),
        appearance: Appearance::Light,
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
