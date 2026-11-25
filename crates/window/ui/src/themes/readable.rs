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

#[cfg(test)]
mod tests {
    use gpui_kit::Hsla;

    use super::{DISTINCT, READABLE};
    use crate::colour::{composited, contrast, hex};
    use crate::themes::{Appearance, LIGHT, Themes};
    use crate::tokens::{Palette, TOKEN_NAMES};

    fn token(palette: &Palette, name: &str) -> Hsla {
        palette
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` is no token"))
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
}
