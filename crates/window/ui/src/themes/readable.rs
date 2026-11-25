/// Pairs of a text token and the ground it is drawn on. Each reads at least 4.5 to 1, with a
/// translucent ground laid over `pane` first.
pub const READABLE: &[(&str, &str)] = &[
    ("text", "window"),
    ("text", "pane"),
    ("text", "raised"),
    ("text", "selection"),
    ("text", "add_ground"),
    ("text", "del_ground"),
    ("text", "add_word"),
    ("text", "del_word"),
    ("text_muted", "window"),
    ("text_muted", "pane"),
    ("text_muted", "raised"),
    ("text_muted", "add_ground"),
    ("text_muted", "del_ground"),
    ("text_dim", "window"),
    ("text_dim", "pane"),
    ("text_dim", "raised"),
    ("text_dim", "add_ground"),
    ("text_dim", "del_ground"),
    ("text_on_accent", "accent"),
    ("text_on_accent", "accent_hover"),
    ("text_on_warn", "warn"),
    ("link", "pane"),
    ("link", "raised"),
    ("info", "pane"),
    ("info", "raised"),
    ("accent_text", "pane"),
    ("accent_text", "raised"),
    ("ok", "pane"),
    ("ok", "raised"),
    ("warn", "pane"),
    ("warn", "raised"),
    ("danger", "pane"),
    ("danger", "raised"),
    ("add_text", "pane"),
    ("add_text", "add_ground"),
    ("del_text", "pane"),
    ("del_text", "del_ground"),
    ("syn_keyword", "pane"),
    ("syn_keyword", "add_ground"),
    ("syn_keyword", "del_ground"),
    ("syn_function", "pane"),
    ("syn_function", "add_ground"),
    ("syn_function", "del_ground"),
    ("syn_string", "pane"),
    ("syn_string", "add_ground"),
    ("syn_string", "del_ground"),
    ("syn_number", "pane"),
    ("syn_number", "add_ground"),
    ("syn_number", "del_ground"),
    ("syn_comment", "pane"),
    ("syn_comment", "add_ground"),
    ("syn_comment", "del_ground"),
];

/// Pairs of tokens that must not look the same once laid over `pane`.
pub const DISTINCT: &[(&str, &str)] = &[
    ("window", "pane"),
    ("pane", "raised"),
    ("pane", "selection"),
    ("pane", "add_ground"),
    ("pane", "del_ground"),
    ("add_ground", "del_ground"),
    ("text", "text_muted"),
    ("text_muted", "text_dim"),
    ("accent", "accent_hover"),
    ("ok", "danger"),
    ("warn", "danger"),
    ("ok", "warn"),
];

/// Pairs of a toolkit text role and the toolkit role it is drawn on, for the panes that still read
/// the toolkit's colours. Each reads at least 4.5 to 1, with a translucent ground laid over
/// `background` first.
pub const KIT_READABLE: &[(&str, &str)] = &[
    ("foreground", "background"),
    ("muted_foreground", "background"),
    ("muted_foreground", "sidebar"),
    ("muted_foreground", "sidebar_accent"),
    ("muted_foreground", "secondary"),
    ("muted_foreground", "list_hover"),
    ("sidebar_foreground", "list_hover"),
    ("primary_foreground", "primary"),
    ("secondary_foreground", "secondary"),
    ("accent_foreground", "accent"),
    ("warning_foreground", "warning"),
    ("sidebar_foreground", "sidebar"),
    ("sidebar_accent_foreground", "sidebar_accent"),
    ("foreground", "secondary"),
    ("foreground", "list_hover"),
    ("danger", "background"),
    ("foreground", "muted"),
    ("danger", "muted"),
    ("success", "background"),
    ("success", "muted"),
    ("success", "secondary"),
    ("warning", "background"),
    ("warning", "muted"),
    ("warning", "secondary"),
];

/// Pairs of toolkit roles that must not look the same once laid over `background`.
pub const KIT_DISTINCT: &[(&str, &str)] = &[
    ("background", "secondary"),
    ("background", "selection"),
    ("background", "list_hover"),
    ("background", "border"),
    ("background", "primary"),
    ("secondary", "list_hover"),
    ("success", "danger"),
    ("warning", "danger"),
    ("success", "warning"),
    ("foreground", "muted_foreground"),
    ("sidebar", "sidebar_accent"),
];

#[cfg(test)]
mod tests {
    use gpui_kit::Hsla;

    use gpui_kit::component::theme::{Theme, ThemeColor};

    use super::{DISTINCT, KIT_DISTINCT, KIT_READABLE, READABLE};
    use crate::colour::{composited, contrast, hex};
    use crate::themes::{Appearance, KIT_ROLES, LIGHT, Themes, kit_role};
    use crate::tokens::{Palette, TOKEN_NAMES};

    fn token(palette: &Palette, name: &str) -> Hsla {
        palette
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` is no token"))
    }

    fn kit_colours(theme: &crate::themes::ThemeDefinition) -> ThemeColor {
        let mut kit = Theme::default();
        theme.apply_to_kit(&mut kit);
        kit.colors
    }

    fn kit(colors: &ThemeColor, name: &str) -> Hsla {
        kit_role(colors, name).unwrap_or_else(|| panic!("`{name}` is no kit role"))
    }

    fn channels(colour: Hsla) -> [u8; 3] {
        let rgba = colour.to_rgb();
        [rgba.r, rgba.g, rgba.b].map(|channel| (channel * 255.0).round() as u8)
    }

    #[test]
    fn every_pair_names_a_token() {
        for (first, second) in READABLE.iter().chain(DISTINCT) {
            for name in [first, second] {
                assert!(TOKEN_NAMES.contains(name), "`{name}` is no token");
            }
        }
    }

    #[test]
    fn in_each_built_in_theme_every_readable_pair_reads_four_and_a_half_to_one() {
        for theme in Themes::built_in().all() {
            let palette = &theme.palette;
            for (text, ground) in READABLE {
                let ratio = contrast(token(palette, text), token(palette, ground), palette.pane);
                assert!(
                    ratio >= 4.5,
                    "{}: `{text}` on `{ground}` reads {ratio:.2} to 1",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn in_each_built_in_theme_the_distinct_pairs_differ() {
        for theme in Themes::built_in().all() {
            let palette = &theme.palette;
            for (first, second) in DISTINCT {
                let shown = |name: &str| channels(composited(token(palette, name), palette.pane));
                assert_ne!(
                    shown(first),
                    shown(second),
                    "{}: `{first}` and `{second}` look the same",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn in_each_built_in_theme_a_raised_ground_shows_on_the_window_and_the_pane() {
        for theme in Themes::built_in().all() {
            let palette = &theme.palette;
            for ground in ["window", "pane"] {
                let ratio = contrast(palette.raised, token(palette, ground), palette.pane);
                assert!(
                    ratio >= 1.15,
                    "{}: `raised` on `{ground}` reads {ratio:.2} to 1",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn the_dimmest_text_of_the_dark_theme_is_the_darkest_grey_that_passes() {
        let themes = Themes::built_in();
        let palette = &themes.named(crate::themes::DARK).expect("built in").palette;
        let [red, green, blue] = channels(palette.text_dim);
        assert!(red == green && green == blue, "text_dim is a grey");
        let darker = hex(u32::from(red - 1) * 0x010101);
        let fails = READABLE.iter().any(|(text, ground)| {
            *text == "text_dim" && contrast(darker, token(palette, ground), palette.pane) < 4.5
        });
        assert!(fails, "a grey one step darker still passes every pair");
    }

    #[test]
    fn the_light_theme_is_primers_light_named_light() {
        let themes = Themes::built_in();
        let theme = themes.named(LIGHT).expect("built in");
        assert_eq!(theme.name, "Light");
        assert_eq!(theme.appearance, Appearance::Light);
        assert_eq!(theme.palette.pane, hex(0xffffff));
        assert_eq!(theme.palette.accent, hex(0x0969da));
    }

    #[test]
    fn each_colour_of_the_light_terminal_but_black_reads_three_to_one_on_its_ground() {
        let themes = Themes::built_in();
        let terminal = &themes.named(LIGHT).expect("built in").terminal;
        for (number, colour) in terminal.ansi.iter().enumerate().skip(1) {
            let ratio = contrast(*colour, terminal.background, terminal.background);
            assert!(ratio >= 3.0, "colour {number} reads {ratio:.2} to 1");
        }
        let shown: Vec<_> = terminal.ansi.iter().copied().map(channels).collect();
        for (number, colour) in shown.iter().enumerate() {
            assert!(
                !shown[..number].contains(colour),
                "colour {number} repeats an earlier one"
            );
        }
    }

    #[test]
    fn the_light_terminals_greys_keep_the_order_light_themes_use_and_stay_apart() {
        let themes = Themes::built_in();
        let terminal = &themes.named(LIGHT).expect("built in").terminal;
        let (ansi, ground) = (&terminal.ansi, terminal.background);
        // The contrast against black rises with the lightness of what it is laid on.
        let lightness = |colour: Hsla| contrast(composited(colour, ground), hex(0x000000), ground);
        assert!(
            lightness(ansi[8]) < lightness(ansi[7]),
            "bright black is darker than white"
        );
        assert!(
            lightness(ansi[7]) < lightness(ansi[15]),
            "white is darker than bright white"
        );
        for (darker, lighter) in [(8, 7), (7, 15)] {
            let apart = contrast(ansi[darker], ansi[lighter], ground);
            assert!(
                apart >= 1.3,
                "colours {darker} and {lighter} are {apart:.2} to 1 apart"
            );
        }
    }

    #[test]
    fn every_kit_pair_names_a_kit_role() {
        for (first, second) in KIT_READABLE.iter().chain(KIT_DISTINCT) {
            for name in [first, second] {
                assert!(KIT_ROLES.contains(name), "`{name}` is no kit role");
            }
        }
    }

    #[test]
    fn in_each_built_in_theme_the_kit_roles_the_panes_read_are_readable() {
        for theme in Themes::built_in().all() {
            let colors = kit_colours(theme);
            for (text, ground) in KIT_READABLE {
                let ratio = contrast(kit(&colors, text), kit(&colors, ground), colors.background);
                assert!(
                    ratio >= 4.5,
                    "{}: kit `{text}` on `{ground}` reads {ratio:.2} to 1",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn in_each_built_in_theme_the_kit_roles_the_panes_read_are_distinct() {
        let mut alike = Vec::new();
        for theme in Themes::built_in().all() {
            let colors = kit_colours(theme);
            for (first, second) in KIT_DISTINCT {
                let shown =
                    |name: &str| channels(composited(kit(&colors, name), colors.background));
                if shown(first) == shown(second) {
                    alike.push(format!("{}: kit `{first}` and `{second}`", theme.name));
                }
            }
        }
        assert!(alike.is_empty(), "these look the same: {alike:?}");
    }

    #[test]
    fn the_sidebars_pairs_of_a_text_role_and_a_ground_are_all_listed_as_readable() {
        for pair in [
            ("sidebar_foreground", "sidebar"),
            ("sidebar_accent_foreground", "sidebar_accent"),
            ("muted_foreground", "sidebar"),
            ("muted_foreground", "sidebar_accent"),
            ("muted_foreground", "list_hover"),
            ("sidebar_foreground", "list_hover"),
            ("muted_foreground", "secondary"),
        ] {
            assert!(
                KIT_READABLE.contains(&pair),
                "{pair:?} is drawn and not listed"
            );
        }
    }

    #[test]
    fn the_pairs_the_panes_and_the_settings_screen_draw_are_all_listed_as_readable() {
        for pair in [
            ("foreground", "background"),
            ("muted_foreground", "background"),
            ("danger", "background"),
            ("foreground", "muted"),
            ("danger", "muted"),
            ("primary_foreground", "primary"),
            ("secondary_foreground", "secondary"),
            ("warning_foreground", "warning"),
            ("success", "background"),
            ("success", "muted"),
            ("success", "secondary"),
            ("warning", "background"),
            ("warning", "muted"),
            ("warning", "secondary"),
            ("muted_foreground", "secondary"),
            ("foreground", "secondary"),
            ("foreground", "list_hover"),
            ("accent_foreground", "accent"),
        ] {
            assert!(
                KIT_READABLE.contains(&pair),
                "{pair:?} is drawn and not listed"
            );
        }
    }

    #[test]
    fn in_each_built_in_theme_the_terminal_draws_its_text_apart_from_its_ground() {
        for theme in Themes::built_in().all() {
            let terminal = &theme.terminal;
            assert_ne!(
                channels(terminal.foreground),
                channels(terminal.background),
                "{}",
                theme.name
            );
        }
    }
}
