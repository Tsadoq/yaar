use gpui_kit::component::theme::{Theme, ThemeColor};
use gpui_kit::*;

use super::ThemeDefinition;
use crate::colour::composited;
use crate::fonts::{CODE as MONO_FONT, UI as FONT};

/// A kit role written with one role of the palette, or one made of two, is a line of this list.
/// `$palette` names the palette inside the values.
macro_rules! kit_roles {
    ($palette:ident; $( $role:ident = $value:expr, )+) => {
        pub const KIT_ROLES: [&str; 38] = [ $( stringify!($role), )+ ];

        /// A kit role by its name, as a theme holds it.
        pub fn kit_role(colors: &ThemeColor, name: &str) -> Option<Hsla> {
            match name {
                $( stringify!($role) => Some(colors.$role), )+
                _ => None,
            }
        }

        impl ThemeDefinition {
            fn write_kit_roles(&self, colors: &mut ThemeColor) {
                let $palette = &self.palette;
                $( colors.$role = $value; )+
            }
        }
    };
}

kit_roles! { p;
    background = p.pane,
    foreground = p.text,
    border = p.edge,
    input = p.edge,
    ring = p.focus,
    caret = p.text,
    selection = p.selection,
    primary = p.accent,
    primary_foreground = p.text_on_accent,
    secondary = p.raised,
    secondary_foreground = p.text,
    muted = p.raised,
    muted_foreground = p.text_muted,
    accent = composited(p.separator, p.raised),
    accent_foreground = p.text,
    list_hover = composited(p.separator, p.raised),
    success = p.ok,
    warning = p.warn,
    warning_foreground = p.text_on_warn,
    danger = p.danger,
    info = p.info,
    sidebar = p.window,
    sidebar_foreground = p.text,
    sidebar_border = p.separator,
    sidebar_accent = p.raised,
    sidebar_accent_foreground = p.text,
    red = p.del_text,
    red_light = p.del_ground,
    green = p.add_text,
    green_light = p.add_ground,
    yellow = p.warn,
    yellow_light = composited(Hsla { a: 0.16, ..p.warn }, p.pane),
    blue = p.link,
    blue_light = p.selection,
    magenta = p.syn_keyword,
    magenta_light = p.raised,
    cyan = p.syn_function,
    cyan_light = p.raised,
}

impl ThemeDefinition {
    /// Writes the toolkit's 38 colour roles, its fonts and its radii. Anything else the toolkit
    /// holds stays as it was.
    pub fn apply_to_kit(&self, theme: &mut Theme) {
        self.write_kit_roles(&mut theme.colors);
        theme.font_family = FONT.into();
        theme.mono_font_family = MONO_FONT.into();
        theme.font_size = px(13.);
        theme.mono_font_size = px(13.);
        theme.radius = px(6.);
        theme.radius_lg = px(10.);
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::theme::{Theme, ThemeColor};
    use gpui_kit::{Hsla, px};

    use super::{KIT_ROLES, kit_role};
    use crate::colour::{composited, hex};
    use crate::themes::{Appearance, ThemeDefinition, Themes};
    use crate::tokens::Palette;

    fn kit_of(theme: &ThemeDefinition) -> Theme {
        let mut kit = Theme::default();
        theme.apply_to_kit(&mut kit);
        kit
    }

    /// The table of the mapping, written again from the theme's own tokens.
    fn wanted(p: &Palette) -> Vec<(&'static str, Hsla)> {
        let over_pane = composited(Hsla { a: 0.16, ..p.warn }, p.pane);
        let hover = composited(p.separator, p.raised);
        vec![
            ("background", p.pane),
            ("foreground", p.text),
            ("border", p.edge),
            ("input", p.edge),
            ("ring", p.focus),
            ("caret", p.text),
            ("selection", p.selection),
            ("primary", p.accent),
            ("primary_foreground", p.text_on_accent),
            ("secondary", p.raised),
            ("secondary_foreground", p.text),
            ("muted", p.raised),
            ("muted_foreground", p.text_muted),
            ("accent", hover),
            ("accent_foreground", p.text),
            ("list_hover", hover),
            ("success", p.ok),
            ("warning", p.warn),
            ("warning_foreground", p.text_on_warn),
            ("danger", p.danger),
            ("info", p.info),
            ("sidebar", p.window),
            ("sidebar_foreground", p.text),
            ("sidebar_border", p.separator),
            ("sidebar_accent", p.raised),
            ("sidebar_accent_foreground", p.text),
            ("red", p.del_text),
            ("red_light", p.del_ground),
            ("green", p.add_text),
            ("green_light", p.add_ground),
            ("yellow", p.warn),
            ("yellow_light", over_pane),
            ("blue", p.link),
            ("blue_light", p.selection),
            ("magenta", p.syn_keyword),
            ("magenta_light", p.raised),
            ("cyan", p.syn_function),
            ("cyan_light", p.raised),
        ]
    }

    #[test]
    fn apply_to_kit_writes_every_kit_role_from_its_token() {
        for theme in Themes::built_in().all() {
            let kit = kit_of(theme);
            let wanted = wanted(&theme.palette);
            assert_eq!(wanted.len(), KIT_ROLES.len());
            for ((name, colour), listed) in wanted.into_iter().zip(KIT_ROLES) {
                assert_eq!(name, listed, "the roles come in the order of the list");
                assert_eq!(
                    kit_role(&kit.colors, name),
                    Some(colour),
                    "{}: kit role `{name}`",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn the_light_of_a_warning_mark_is_its_sixteenth_over_the_pane() {
        for theme in Themes::built_in().all() {
            let palette = &theme.palette;
            let kit = kit_of(theme);
            let shown = kit.colors.yellow_light.to_rgb();
            assert_eq!(shown.a, 1.0, "{}: laid over the pane already", theme.name);
            let [warn, pane] = [palette.warn, palette.pane].map(|c| c.to_rgb());
            let sixteenth = |warn: f32, pane: f32| warn * 0.16 + pane * 0.84;
            for (got, want) in [
                (shown.r, sixteenth(warn.r, pane.r)),
                (shown.g, sixteenth(warn.g, pane.g)),
                (shown.b, sixteenth(warn.b, pane.b)),
            ] {
                assert!(
                    (got - want).abs() < 0.002,
                    "{}: {got} for {want}",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn apply_to_kit_sets_the_fonts_and_radii() {
        for theme in Themes::built_in().all() {
            let kit = kit_of(theme);
            assert_eq!(kit.font_family, "Inter");
            assert_eq!(kit.mono_font_family, "JetBrains Mono");
            assert_eq!(kit.font_size, px(13.));
            assert_eq!(kit.mono_font_size, px(13.));
            assert_eq!(kit.radius, px(6.));
            assert_eq!(kit.radius_lg, px(10.));
        }
    }

    #[test]
    fn a_definition_leaves_the_roles_of_the_toolkit_that_the_list_does_not_name() {
        let mut kit = Theme::default();
        kit.colors.accordion = hex(0x010203);
        kit.shadow = false;
        let mut definition = Themes::built_in().all()[0].clone();
        definition.appearance = Appearance::Dark;
        definition.apply_to_kit(&mut kit);
        assert_eq!(kit.colors.accordion, hex(0x010203));
        assert!(!kit.shadow);
    }

    #[test]
    fn the_kit_input_shows_its_caret_selection_and_border_on_its_ground() {
        for theme in Themes::built_in().all() {
            let colors = kit_of(theme).colors;
            let shown = |colour: Hsla| composited(colour, colors.background);
            for (name, colour) in [
                ("caret", colors.caret),
                ("selection", colors.selection),
                ("border", colors.border),
            ] {
                assert_ne!(
                    shown(colour),
                    shown(colors.background),
                    "{}: the kit's `{name}` is the input's own ground",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn a_name_that_is_no_kit_role_reads_nothing() {
        assert_eq!(kit_role(&ThemeColor::default(), "pane"), None);
        assert!(KIT_ROLES.contains(&"sidebar_accent"));
    }
}
