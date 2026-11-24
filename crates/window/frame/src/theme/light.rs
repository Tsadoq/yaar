use super::{Appearance, Colors, TerminalColors, ThemeDefinition, hex, hexa};

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
        // Darkened from the toolkit's #737373 (3.76 to 1 on `sidebar_accent` and `secondary`) to
        // 4.5 to 1.
        muted_foreground: hex(0x666666),
        accent: hex(0xf5f5f5),
        accent_foreground: hex(0x171717),
        list_hover: hex(0xf5f5f5),
        // Darkened from the toolkit's #22c55e (2.28 to 1 or less on `background`, `muted`,
        // `secondary`) to 4.5 to 1 on each.
        success: hex(0x136e35),
        // Darkened from the toolkit's #eab308 (1.92 to 1 or less on the same three) to 4.5 to 1 on
        // each.
        warning: hex(0x6f5504),
        warning_foreground: hex(0xfafafa),
        // Darkened from the toolkit's #ef4444 (3.76 and 3.45 to 1 on `background` and `muted`) to
        // 4.5 to 1.
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
            // Each colour under 3 to 1 on white is darkened to the first step that reads 3 to 1.
            // The greys 7, 8, 15 are set by hand: bright black darker than white, white darker than
            // bright white, each 1.3 to 1 or more apart.
            ansi: [
                hex(0x000000),
                colors.red,
                colors.green,
                hex(0xb17904),
                colors.blue,
                colors.magenta,
                colors.cyan,
                hex(0x737373),
                hex(0x595959),
                hex(0xf75959),
                hex(0x1ea44f),
                hex(0xa58504),
                hex(0x2f89f8),
                hex(0xb36bfb),
                hex(0x0e9fb6),
                hex(0x8c8c8c),
            ],
        },
        colors,
    }
}
