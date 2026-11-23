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
        // The toolkit's #737373 reads 3.76 to 1 on the selected row's ground (`sidebar_accent`)
        // and on `secondary`, which has the same value: darkened until 4.5 to 1.
        muted_foreground: hex(0x666666),
        accent: hex(0xf5f5f5),
        accent_foreground: hex(0x171717),
        list_hover: hex(0xf5f5f5),
        // The toolkit's #22c55e reads 2.28, 2.09 and 1.81 to 1 on `background`, `muted` and
        // `secondary`, where the panes draw it as words: darkened until 4.5 to 1 on each.
        success: hex(0x136e35),
        // The toolkit's #eab308 reads 1.92, 1.76 and 1.52 to 1 on the same three: darkened
        // until 4.5 to 1 on each.
        warning: hex(0x6f5504),
        // The toolkit's own #fafafa: it reads 6.8 to 1 on the darkened `warning` (it read 1.84
        // to 1 on the toolkit's yellow, which is why this role was once darker).
        warning_foreground: hex(0xfafafa),
        // The toolkit's #ef4444 reads 3.76 to 1 on `background` and 3.45 to 1 on `muted`:
        // darkened until 4.5 to 1 on both.
        danger: hex(0xd41212),
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
            // What the pane drew before, but where that read under 3 to 1 on this white ground
            // (the numbers noted): darkened in steps of 5 points of lightness to the first
            // that reads 3 to 1 and is no earlier colour of the palette. The three greys, 7, 8
            // and 15, are set by hand instead, in the order light terminal themes use: bright
            // black darker than white, and white darker than bright white, each 1.3 to 1 or
            // more from the next. The steps alone gave bright black and bright white one level
            // apart, and bright white darker than white.
            ansi: [
                hex(0x000000),
                colors.red,
                colors.green,
                // Colour 3, the yellow, was 2.94 to 1.
                hex(0xb17904),
                colors.blue,
                colors.magenta,
                colors.cyan,
                // Colour 7, white, was 1.26 to 1; it reads 4.7 to 1 now, and colour 8, bright
                // black, 7.0 to 1.
                hex(0x737373),
                hex(0x595959),
                // Colours 9 to 14, the bright ones, were 2.77, 1.74, 1.53, 2.54, 2.64 and 1.81
                // to 1.
                hex(0xf75959),
                hex(0x1ea44f),
                hex(0xa58504),
                hex(0x2f89f8),
                hex(0xb36bfb),
                hex(0x0e9fb6),
                // Colour 15, bright white, was 1.0 to 1; it reads 3.4 to 1 now.
                hex(0x8c8c8c),
            ],
        },
        colors,
    }
}
