//! Every colour the Window draws is a role of a `ThemeDefinition` here. A role under 4.5 to 1 on
//! its ground is moved away from the ground's lightness in 5-point steps. Light terminal greys 7, 8
//! and 15 are set by hand: 8 darker than 7, 7 darker than 15.

mod dark;
mod light;

use gpui_kit::component::theme::ThemeColor;
use gpui_kit::*;

pub use dark::dark;
pub use light::light;

pub const LIGHT: &str = "Light";

pub const DARK: &str = "Dark";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        }
    }
}

pub fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

/// xterm's colours: a 6 by 6 by 6 cube from 16 (a channel is 0, or 55 plus 40 a step), then 24
/// greys from 232 (8 plus 10 a step). Under 16 this gives xterm's own sixteen; the terminal pane
/// takes those from the theme.
pub fn indexed(index: u8) -> Hsla {
    const BASE: [u32; 16] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
        0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    let step = |n: u8| if n == 0 { 0 } else { 55 + 40 * u32::from(n) };
    match index {
        0..=15 => hex(BASE[usize::from(index)]),
        16..=231 => {
            let n = index - 16;
            hex(step(n / 36) << 16 | step(n / 6 % 6) << 8 | step(n % 6))
        }
        232..=255 => {
            let grey = 8 + 10 * u32::from(index - 232);
            hex(grey << 16 | grey << 8 | grey)
        }
    }
}

pub fn rgb24(red: u8, green: u8, blue: u8) -> Hsla {
    hex(u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue))
}

macro_rules! roles {
    ($( $(#[$doc:meta])* $role:ident, )+) => {
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct Colors {
            $( $(#[$doc])* pub $role: Hsla, )+
        }

        impl Colors {
            pub fn onto(&self, kit: &mut ThemeColor) {
                $( kit.$role = self.$role; )+
            }

            pub fn of(kit: &ThemeColor) -> Colors {
                Colors { $( $role: kit.$role, )+ }
            }

            pub fn roles(&self) -> Vec<(&'static str, Hsla)> {
                vec![ $( (stringify!($role), self.$role), )+ ]
            }
        }
    };
}

roles! {
    background,
    foreground,
    border,
    input,
    ring,
    caret,
    selection,
    primary,
    primary_foreground,
    secondary,
    secondary_foreground,
    muted,
    muted_foreground,
    accent,
    accent_foreground,
    list_hover,
    success,
    warning,
    warning_foreground,
    danger,
    info,
    sidebar,
    sidebar_foreground,
    sidebar_border,
    sidebar_accent,
    sidebar_accent_foreground,
    /// Red, as a mark. `red` to `cyan_light` are also the dark theme's terminal colours 1 to 6 and
    /// 9 to 14; the light theme replaces those that read under 3 to 1 on white.
    red,
    red_light,
    green,
    green_light,
    yellow,
    yellow_light,
    blue,
    blue_light,
    magenta,
    magenta_light,
    cyan,
    cyan_light,
}

pub const READABLE: &[(&str, &str)] = &[
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

pub const DISTINCT: &[(&str, &str)] = &[
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalColors {
    pub foreground: Hsla,
    pub background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub ansi: [Hsla; 16],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeDefinition {
    pub name: String,
    /// The toolkit's mode it is applied in, and the window's own appearance on macOS while it is
    /// chosen.
    pub appearance: Appearance,
    pub colors: Colors,
    pub terminal: TerminalColors,
}

impl ThemeDefinition {
    pub fn onto(&self, kit: &mut ThemeColor) {
        self.colors.onto(kit);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeError {
    /// Its name is empty, longer than 64 characters, or holds a control character.
    BadName(String),
    Taken(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Themes {
    definitions: Vec<ThemeDefinition>,
}

impl Themes {
    pub fn built_in() -> Themes {
        Themes {
            definitions: vec![light(), dark()],
        }
    }

    pub fn with(mut self, definition: ThemeDefinition) -> Result<Themes, ThemeError> {
        let name = &definition.name;
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(ThemeError::BadName(definition.name));
        }
        if self.named(name).is_some() {
            return Err(ThemeError::Taken(definition.name));
        }
        self.definitions.push(definition);
        Ok(self)
    }

    pub fn all(&self) -> &[ThemeDefinition] {
        &self.definitions
    }

    pub fn named(&self, name: &str) -> Option<&ThemeDefinition> {
        let name = name.to_lowercase();
        self.definitions
            .iter()
            .find(|definition| definition.name.to_lowercase() == name)
    }

    pub fn resolve(&self, choice: &yaar_link::ThemeChoice, system: Appearance) -> &ThemeDefinition {
        if let yaar_link::ThemeChoice::Named(name) = choice
            && let Some(definition) = self.named(name)
        {
            return definition;
        }
        // Index 0 is the light theme and 1 the dark one: `built_in` puts them first and `with` only
        // appends.
        match system {
            Appearance::Light => &self.definitions[0],
            Appearance::Dark => &self.definitions[1],
        }
    }
}

struct Applied(ThemeDefinition);

impl Global for Applied {}

pub fn applied(cx: &App) -> ThemeDefinition {
    cx.try_global::<Applied>()
        .map_or_else(light, |applied| applied.0.clone())
}

/// Only `crate::Looks` may call this; it does not touch the toolkit's colours.
pub(crate) fn set_applied(definition: ThemeDefinition, cx: &mut App) {
    cx.set_global(Applied(definition));
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::component::theme::{Theme, ThemeColor, ThemeMode};
    use yaar_link::ThemeChoice;

    use super::{
        Appearance, Colors, DARK, DISTINCT, LIGHT, READABLE, TerminalColors, ThemeDefinition,
        ThemeError, Themes, applied, dark, hex, hexa, indexed, light, rgb24, set_applied,
    };
    use gpui_kit::Hsla;

    fn sepia() -> ThemeDefinition {
        ThemeDefinition {
            name: "Sepia".to_owned(),
            appearance: Appearance::Light,
            colors: Colors {
                background: hex(0xf4ecd8),
                foreground: hex(0x433422),
                border: hex(0xd8c9a8),
                input: hex(0xd2c29d),
                ring: hex(0xa8946a),
                caret: hex(0x5b4636),
                selection: hexa(0xb89b5e66),
                primary: hex(0x704214),
                primary_foreground: hex(0xfbf3e0),
                secondary: hex(0xe6dabc),
                secondary_foreground: hex(0x3a2b1a),
                muted: hex(0xece1c5),
                muted_foreground: hex(0x6e5a3e),
                accent: hex(0xe0d2ae),
                accent_foreground: hex(0x35281a),
                list_hover: hex(0xeadfc3),
                success: hex(0x5a7d2a),
                warning: hex(0xc58a1b),
                warning_foreground: hex(0x2b1d0b),
                danger: hex(0x9c2f1f),
                info: hex(0x2e6f7a),
                sidebar: hex(0xeee4cb),
                sidebar_foreground: hex(0x3d2f1f),
                sidebar_border: hex(0xd5c6a4),
                sidebar_accent: hex(0xdfd0ab),
                sidebar_accent_foreground: hex(0x2f2315),
                red: hex(0xa63d2f),
                red_light: hex(0xc4604f),
                green: hex(0x56742c),
                green_light: hex(0x7a9a4a),
                yellow: hex(0xa8791d),
                yellow_light: hex(0xcc9a33),
                blue: hex(0x35607f),
                blue_light: hex(0x5a85a6),
                magenta: hex(0x7d4a73),
                magenta_light: hex(0xa0709a),
                cyan: hex(0x3a7a74),
                cyan_light: hex(0x5fa09a),
            },
            terminal: TerminalColors {
                foreground: hex(0x433422),
                background: hex(0xf4ecd8),
                cursor: hex(0x704214),
                selection: hexa(0xb89b5e66),
                ansi: [
                    hex(0x1a120a),
                    hex(0xa63d2f),
                    hex(0x56742c),
                    hex(0xa8791d),
                    hex(0x35607f),
                    hex(0x7d4a73),
                    hex(0x3a7a74),
                    hex(0xd9cdb0),
                    hex(0x7a6a52),
                    hex(0xc4604f),
                    hex(0x7a9a4a),
                    hex(0xcc9a33),
                    hex(0x5a85a6),
                    hex(0xa0709a),
                    hex(0x5fa09a),
                    hex(0xfffaf0),
                ],
            },
        }
    }

    fn channels(colour: Hsla) -> [u8; 4] {
        let rgba = colour.to_rgb();
        [rgba.r, rgba.g, rgba.b, rgba.a].map(|channel| (channel * 255.0).round() as u8)
    }

    fn role(colors: &Colors, name: &str) -> Hsla {
        colors
            .roles()
            .into_iter()
            .find(|(role, _)| *role == name)
            .unwrap_or_else(|| panic!("`{name}` is no role of Colors"))
            .1
    }

    fn luminance(colour: Hsla) -> f32 {
        let rgba = colour.to_rgb();
        let linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
    }

    fn composited(colour: Hsla, ground: Hsla) -> Hsla {
        let (over, under) = (colour.to_rgb(), ground.to_rgb());
        let mix = |a: f32, b: f32| a * over.a + b * (1.0 - over.a);
        gpui_kit::Rgba {
            r: mix(over.r, under.r),
            g: mix(over.g, under.g),
            b: mix(over.b, under.b),
            a: 1.0,
        }
        .into()
    }

    fn contrast(text: Hsla, ground: Hsla, backdrop: Hsla) -> f32 {
        let (a, b) = (
            luminance(composited(text, backdrop)),
            luminance(composited(ground, backdrop)),
        );
        let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
        (lighter + 0.05) / (darker + 0.05)
    }

    fn rounded(colour: Hsla) -> Hsla {
        let [r, g, b, _] = channels(colour);
        hex(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }

    /// Moves `start` away from the lightness of `ground` by 5 points of HSL lightness a step, to
    /// the first step that `passes`; the other way when none does.
    fn moved(start: Hsla, ground: Hsla, passes: impl Fn(Hsla) -> bool) -> Hsla {
        let away = if start.l > ground.l { 1.0 } else { -1.0 };
        for direction in [away, -away] {
            for step in 0..=20 {
                let lightness = start.l + direction * 0.05 * step as f32;
                if !(-1e-4..=1.0 + 1e-4).contains(&lightness) {
                    break;
                }
                let candidate = rounded(Hsla {
                    l: lightness.clamp(0.0, 1.0),
                    ..start
                });
                if passes(candidate) {
                    return candidate;
                }
            }
        }
        panic!("no step of 5 points makes {start:?} pass");
    }

    #[test]
    fn a_third_theme_written_as_one_definition_is_accepted_and_applied_by_name() {
        let themes = Themes::built_in().with(sepia()).expect("a new name");
        assert_eq!(
            themes
                .all()
                .iter()
                .map(|theme| theme.name.as_str())
                .collect::<Vec<_>>(),
            [LIGHT, DARK, "Sepia"]
        );
        assert_eq!(themes.named("sEPia"), Some(&sepia()));
        for system in [Appearance::Light, Appearance::Dark] {
            assert_eq!(
                themes.resolve(&ThemeChoice::Named("Sepia".to_owned()), system),
                &sepia(),
                "a chosen theme is drawn whatever the system says"
            );
        }
        assert_eq!(
            themes.resolve(&ThemeChoice::Named("SEPIA".to_owned()), Appearance::Dark),
            &sepia()
        );

        let mut kit = ThemeColor::default();
        kit.accordion = hex(0x010203);
        sepia().onto(&mut kit);
        let given = Colors::of(&kit).roles();
        let wanted = sepia().colors.roles();
        assert_eq!(given.len(), wanted.len());
        for ((name, given), (wanted_name, wanted)) in given.into_iter().zip(wanted) {
            assert_eq!(name, wanted_name);
            assert_eq!(given, wanted, "role `{name}` as the toolkit holds it");
        }
        assert_eq!(kit.accordion, hex(0x010203), "a role of the toolkit's own");
    }

    #[test]
    fn a_definition_is_refused_for_a_name_taken_or_not_a_name_and_the_themes_stay() {
        let themes = Themes::built_in().with(sepia()).expect("a new name");
        let named = |name: &str| ThemeDefinition {
            name: name.to_owned(),
            ..sepia()
        };
        assert_eq!(
            themes.clone().with(sepia()),
            Err(ThemeError::Taken("Sepia".to_owned()))
        );
        assert_eq!(
            themes.clone().with(named("SEPIA")),
            Err(ThemeError::Taken("SEPIA".to_owned())),
            "case aside"
        );
        assert_eq!(
            Themes::built_in().with(named("dark")),
            Err(ThemeError::Taken("dark".to_owned()))
        );
        for bad in [
            String::new(),
            "x".repeat(65),
            "two\nlines".to_owned(),
            "tab\there".to_owned(),
            "\u{1b}[31m".to_owned(),
            "bell\u{7}".to_owned(),
            "\u{7f}".to_owned(),
        ] {
            assert_eq!(
                themes.clone().with(named(&bad)),
                Err(ThemeError::BadName(bad.clone()))
            );
        }
        let longest = "x".repeat(64);
        let longest = Themes::built_in()
            .with(named(&longest))
            .expect("64 is fine");
        assert_eq!(longest.all().len(), 3);
        // Characters, not bytes: 64 two-byte characters is a valid name.
        let wide = "é".repeat(64);
        assert!(Themes::built_in().with(named(&wide)).is_ok());
    }

    #[test]
    fn a_choice_that_names_no_theme_or_none_is_drawn_as_the_systems_appearance() {
        let themes = Themes::built_in().with(sepia()).expect("a new name");
        for choice in [
            ThemeChoice::System,
            ThemeChoice::Named("Nowhere".to_owned()),
        ] {
            assert_eq!(themes.resolve(&choice, Appearance::Dark), &dark());
            assert_eq!(themes.resolve(&choice, Appearance::Light), &light());
        }
        assert_eq!(
            themes.resolve(&ThemeChoice::Named(DARK.to_owned()), Appearance::Light),
            &dark()
        );
        assert_eq!(
            themes.resolve(&ThemeChoice::Named(LIGHT.to_owned()), Appearance::Dark),
            &light()
        );
        let night = ThemeDefinition {
            name: "Night".to_owned(),
            appearance: Appearance::Dark,
            ..sepia()
        };
        let themes = themes.with(night).expect("a new name");
        assert_eq!(
            themes.resolve(&ThemeChoice::System, Appearance::Dark),
            &dark()
        );
    }

    #[gpui_kit::test]
    fn the_applied_definition_is_the_light_one_until_one_is_set_and_then_that_one(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            assert_eq!(applied(cx), light());
            set_applied(sepia(), cx);
            assert_eq!(applied(cx), sepia());
            set_applied(dark(), cx);
            assert_eq!(applied(cx), dark());
        });
    }

    #[test]
    fn every_role_has_its_own_name_and_the_lists_of_pairs_name_roles() {
        let roles = light().colors.roles();
        assert_eq!(roles.len(), 38);
        let mut names: Vec<_> = roles.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 38, "no name twice");
        for (first, second) in READABLE.iter().chain(DISTINCT) {
            for name in [first, second] {
                assert!(names.contains(name), "`{name}` is no role");
            }
        }
    }

    #[test]
    fn in_each_built_in_theme_every_text_role_is_readable_on_its_ground() {
        for theme in [light(), dark()] {
            let colors = &theme.colors;
            for (text, ground) in READABLE {
                let ratio = contrast(role(colors, text), role(colors, ground), colors.background);
                assert!(
                    ratio >= 4.5,
                    "{}: `{text}` on `{ground}` is {ratio:.2} to 1",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn the_contrast_a_test_reads_is_wcags_and_sees_what_is_not_readable() {
        let (black, white) = (hex(0x000000), hex(0xffffff));
        assert!((contrast(black, white, white) - 21.0).abs() < 0.01);
        assert!((contrast(white, black, white) - 21.0).abs() < 0.01);
        assert!((contrast(white, white, black) - 1.0).abs() < 0.001);
        // WCAG's own tables give #777777 on white 4.48 to 1.
        assert!((contrast(hex(0x777777), white, white) - 4.48).abs() < 0.01);
        // Half black over white is mid grey: white text on it reads 4.0 to 1.
        let half = hexa(0x00000080);
        assert!((contrast(white, half, white) - 4.0).abs() < 0.02);
        // The toolkit's own white-on-yellow pair, which the light theme had to change.
        assert!(contrast(hex(0xfafafa), hex(0xeab308), white) < 4.5);
    }

    #[test]
    fn in_each_built_in_theme_the_roles_that_must_differ_do_and_so_do_the_terminals_colours() {
        for theme in [light(), dark()] {
            for (first, second) in DISTINCT {
                assert_ne!(
                    role(&theme.colors, first),
                    role(&theme.colors, second),
                    "{}: `{first}` and `{second}`",
                    theme.name
                );
            }
            let ansi = theme.terminal.ansi.map(channels);
            for (number, colour) in ansi.iter().enumerate() {
                assert!(
                    !ansi[..number].contains(colour),
                    "{}: colour {number} repeats an earlier one",
                    theme.name
                );
            }
            assert_ne!(theme.terminal.foreground, theme.terminal.background);
        }
        assert_eq!(
            (light().name.as_str(), light().appearance),
            (LIGHT, Appearance::Light)
        );
        assert_eq!(
            (dark().name.as_str(), dark().appearance),
            (DARK, Appearance::Dark)
        );
    }

    #[test]
    fn the_terminal_of_a_built_in_theme_takes_its_ground_and_marks_from_its_roles() {
        for theme in [light(), dark()] {
            let (colors, terminal) = (&theme.colors, &theme.terminal);
            assert_eq!(terminal.foreground, colors.foreground);
            assert_eq!(terminal.background, colors.background);
            assert_eq!(terminal.cursor, colors.foreground);
            assert_eq!(terminal.selection, colors.selection);
        }
    }

    #[gpui_kit::test]
    fn the_built_in_themes_are_the_toolkits_own_colours_but_for_the_roles_made_readable_by_the_rule(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        for (mode, theme, changed) in [
            (
                ThemeMode::Light,
                light(),
                &["danger", "muted_foreground", "success", "warning"][..],
            ),
            (ThemeMode::Dark, dark(), &["warning_foreground"][..]),
        ] {
            let kit = cx.update(|cx| {
                Theme::change(mode, None, cx);
                Colors::of(&cx.global::<Theme>().colors)
            });
            for ((name, own), (_, kits)) in theme.colors.roles().into_iter().zip(kit.roles()) {
                if !changed.contains(&name) {
                    assert_eq!(channels(own), channels(kits), "`{name}` as the kit has it");
                    continue;
                }
                assert_ne!(channels(own), channels(kits), "`{name}` was changed");
                let grounds: Vec<Hsla> = READABLE
                    .iter()
                    .filter(|(text, _)| *text == name)
                    .map(|(_, ground)| {
                        composited(role(&theme.colors, ground), theme.colors.background)
                    })
                    .collect();
                let readable = |colour: Hsla| {
                    grounds
                        .iter()
                        .all(|ground| contrast(colour, *ground, theme.colors.background) >= 4.5)
                };
                let backdrop = theme.colors.background;
                let worst = grounds
                    .iter()
                    .min_by(|a, b| {
                        let (a, b) = (contrast(kits, **a, backdrop), contrast(kits, **b, backdrop));
                        a.total_cmp(&b)
                    })
                    .expect("a ground for a changed role");
                assert_eq!(
                    channels(own),
                    channels(moved(kits, *worst, readable)),
                    "{}: `{name}` is the first step of 5 points that reads",
                    theme.name
                );
            }
        }
    }

    /// The default palette: xterm's own black, white, bright black and bright white, and the
    /// theme's six and their bright ones.
    fn drawn_before(colors: &Colors) -> [Hsla; 16] {
        [
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
        ]
    }

    #[test]
    fn the_dark_terminal_keeps_the_palette_the_pane_drew_before() {
        let theme = dark();
        assert_eq!(theme.terminal.ansi, drawn_before(&theme.colors));
    }

    const GREYS: [usize; 3] = [7, 8, 15];

    #[test]
    fn the_light_terminals_greys_are_set_by_hand_in_the_order_light_themes_use_and_apart() {
        let theme = light();
        let ansi = &theme.terminal.ansi;
        assert_eq!(
            [channels(ansi[8]), channels(ansi[7]), channels(ansi[15])],
            [
                [0x59, 0x59, 0x59, 255],
                [0x73, 0x73, 0x73, 255],
                [0x8c, 0x8c, 0x8c, 255],
            ],
            "bright black, white and bright white"
        );
        let ground = theme.terminal.background;
        let lightness = |colour: Hsla| luminance(composited(colour, ground));
        assert!(
            lightness(ansi[8]) < lightness(ansi[7]),
            "bright black darker than white"
        );
        assert!(
            lightness(ansi[7]) < lightness(ansi[15]),
            "white darker than bright white"
        );
        for (darker, lighter) in [(8, 7), (7, 15)] {
            let apart = contrast(ansi[darker], ansi[lighter], ground);
            assert!(
                apart >= 1.3,
                "colours {darker} and {lighter} are {apart:.2} to 1 apart"
            );
        }
        for number in GREYS {
            let ratio = contrast(ansi[number], ground, ground);
            assert!(ratio >= 3.0, "colour {number} reads {ratio:.2} to 1");
        }
    }

    #[test]
    fn each_colour_of_the_light_terminal_but_black_reads_three_to_one_on_its_ground() {
        let theme = light();
        let (terminal, before) = (&theme.terminal, drawn_before(&theme.colors));
        assert_eq!(terminal.ansi[0], hex(0x000000), "colour 0 is as it was");
        let mut expected: Vec<Hsla> = vec![before[0]];
        for start in &before[1..] {
            let taken = expected.iter().copied().map(channels).collect::<Vec<_>>();
            expected.push(moved(*start, terminal.background, |candidate| {
                contrast(candidate, terminal.background, terminal.background) >= 3.0
                    && !taken.contains(&channels(candidate))
            }));
        }
        for (number, (colour, wanted)) in terminal.ansi.iter().zip(&expected).enumerate().skip(1) {
            let ratio = contrast(*colour, terminal.background, terminal.background);
            assert!(ratio >= 3.0, "colour {number} reads {ratio:.2} to 1");
            if GREYS.contains(&number) {
                continue;
            }
            assert_eq!(
                channels(*colour),
                channels(*wanted),
                "colour {number} is the first step of 5 points that reads 3 to 1 and is no \
                 earlier colour"
            );
        }
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
            assert!(READABLE.contains(&pair), "{pair:?} is drawn and not listed");
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
            assert!(READABLE.contains(&pair), "{pair:?} is drawn and not listed");
        }
    }

    #[test]
    fn the_numbered_colours_above_the_sixteen_are_xterms_and_a_value_is_itself() {
        assert_eq!(indexed(16), hex(0x000000));
        assert_eq!(indexed(21), hex(0x0000ff));
        assert_eq!(indexed(196), hex(0xff0000));
        assert_eq!(indexed(231), hex(0xffffff));
        assert_eq!(indexed(17), hex(0x00005f));
        assert_eq!(indexed(16 + 36), hex(0x5f0000));
        assert_eq!(indexed(16 + 6 * 2 + 3), hex(0x0087af));
        assert_eq!(indexed(232), hex(0x080808));
        assert_eq!(indexed(244), hex(0x808080));
        assert_eq!(indexed(255), hex(0xeeeeee));
        assert_eq!(indexed(1), hex(0xcd0000));
        assert_eq!(indexed(15), hex(0xffffff));
        assert_eq!(rgb24(0x12, 0x34, 0x56), hex(0x123456));
    }
}
