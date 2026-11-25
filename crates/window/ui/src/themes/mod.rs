mod file;
mod kit;
mod readable;

use std::sync::OnceLock;

use gpui_kit::*;

pub use file::ThemeFileError;
pub use kit::{KIT_ROLES, kit_role};
pub use readable::{DISTINCT, KIT_DISTINCT, KIT_READABLE, READABLE};

use crate::tokens::{Palette, TerminalColors};

pub const LIGHT: &str = "Light";

pub const DARK: &str = "Dark";

/// The built-in theme files, the light theme first and the dark one second.
const BUILT_IN: &[&str] = &[
    include_str!("../../themes/light.toml"),
    include_str!("../../themes/dark.toml"),
];

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

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeDefinition {
    pub name: String,
    /// The toolkit's mode it is applied in, and the window's own appearance on macOS while it is
    /// chosen.
    pub appearance: Appearance,
    pub palette: Palette,
    pub terminal: TerminalColors,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeError {
    /// Its name is empty, longer than 64 characters, or holds a control character.
    BadName(String),
    Taken(String),
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Themes {
    definitions: Vec<ThemeDefinition>,
}

impl Themes {
    pub fn built_in() -> Themes {
        let definitions = BUILT_IN
            .iter()
            .map(|text| ThemeDefinition::from_toml(text).expect("a built-in theme file parses"))
            .collect();
        Themes { definitions }
    }

    pub fn with(mut self, definition: ThemeDefinition) -> Result<Themes, ThemeError> {
        if !valid_name(&definition.name) {
            return Err(ThemeError::BadName(definition.name));
        }
        if self.named(&definition.name).is_some() {
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

    pub fn resolve(&self, chosen: Option<&str>, system: Appearance) -> &ThemeDefinition {
        if let Some(name) = chosen
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

/// What is applied until a theme is.
pub fn built_in_light() -> &'static ThemeDefinition {
    static LIGHT_THEME: OnceLock<ThemeDefinition> = OnceLock::new();
    LIGHT_THEME.get_or_init(|| Themes::built_in().resolve(None, Appearance::Light).clone())
}

pub fn applied(cx: &App) -> ThemeDefinition {
    cx.try_global::<Applied>()
        .map_or_else(|| built_in_light().clone(), |applied| applied.0.clone())
}

pub fn palette(cx: &App) -> Palette {
    cx.try_global::<Applied>()
        .map_or(built_in_light().palette, |applied| applied.0.palette)
}

pub fn set_applied(definition: ThemeDefinition, cx: &mut App) {
    cx.set_global(Applied(definition));
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::{
        Appearance, DARK, LIGHT, ThemeDefinition, ThemeError, Themes, applied, palette, set_applied,
    };
    use crate::colour::{hex, hexa};

    const DARK_FILE: &str = include_str!("../../themes/dark.toml");

    fn sepia() -> ThemeDefinition {
        let text = DARK_FILE
            .replace("name = \"Dark\"", "name = \"Sepia\"")
            .replace("appearance = \"dark\"", "appearance = \"light\"")
            .replace("window = \"#131313\"", "window = \"#f4ecd8\"");
        ThemeDefinition::from_toml(&text).expect("a whole file")
    }

    fn dark() -> ThemeDefinition {
        Themes::built_in().named(DARK).expect("built in").clone()
    }

    fn light() -> ThemeDefinition {
        Themes::built_in().named(LIGHT).expect("built in").clone()
    }

    #[test]
    fn every_built_in_theme_file_parses() {
        for text in super::BUILT_IN {
            assert!(ThemeDefinition::from_toml(text).is_ok());
        }
        let themes = Themes::built_in();
        let names: Vec<_> = themes
            .all()
            .iter()
            .map(|theme| theme.name.as_str())
            .collect();
        assert_eq!(names, [LIGHT, DARK]);
    }

    #[test]
    fn the_dark_theme_holds_the_mocks_colours() {
        let palette = dark().palette;
        assert_eq!(palette.pane, hex(0x171717));
        assert_eq!(palette.accent, hex(0x1447e6));
        assert_eq!(palette.add_ground, hex(0x16281f));
        assert_eq!(palette.del_word, hex(0x5a2a2e));
        assert_eq!(palette.syn_keyword, hex(0xf092c0));
        assert_eq!(palette.overlay, hexa(0x0000008c));
        assert_eq!(palette.syn_comment, palette.text_dim);
    }

    #[test]
    fn the_dark_terminal_keeps_the_sixteen_colours_it_drew_before() {
        let terminal = dark().terminal;
        let before = [
            0x000000, 0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xe5e5e5,
            0x7f7f7f, 0xfca5a5, 0x86efac, 0xfde047, 0x93c5fd, 0xd8b4fe, 0x67e8f9, 0xffffff,
        ];
        assert_eq!(terminal.ansi, before.map(hex));
        assert_eq!(terminal.foreground, hex(0xfafafa));
        assert_eq!(terminal.background, hex(0x171717));
        assert_eq!(terminal.cursor, hex(0xfafafa));
        assert_eq!(terminal.selection, hexa(0x1447e666));
    }

    #[test]
    fn the_terminal_of_a_built_in_theme_takes_its_ground_and_marks_from_its_tokens() {
        for theme in [light(), dark()] {
            let (palette, terminal) = (&theme.palette, &theme.terminal);
            assert_eq!(terminal.foreground, palette.text, "{}", theme.name);
            assert_eq!(terminal.background, palette.pane, "{}", theme.name);
            assert_eq!(terminal.cursor, palette.text, "{}", theme.name);
        }
        let light = light();
        assert_eq!(light.terminal.selection, light.palette.selection);
        // The dark terminal's selection is stronger (40% against 24%): text selected on the terminal's
        // ground must still show.
        let dark = dark();
        assert_ne!(dark.terminal.selection, dark.palette.selection);
        assert_eq!(dark.terminal.selection, hexa(0x1447e666));
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
                themes.resolve(Some("Sepia"), system),
                &sepia(),
                "a chosen theme is drawn whatever the system says"
            );
        }
        assert_eq!(themes.resolve(Some("SEPIA"), Appearance::Dark), &sepia());
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
        for choice in [None, Some("Nowhere")] {
            assert_eq!(themes.resolve(choice, Appearance::Dark), &dark());
            assert_eq!(themes.resolve(choice, Appearance::Light), &light());
        }
        assert_eq!(themes.resolve(Some(DARK), Appearance::Light), &dark());
        assert_eq!(themes.resolve(Some(LIGHT), Appearance::Dark), &light());
        let night = ThemeDefinition {
            name: "Night".to_owned(),
            appearance: Appearance::Dark,
            ..sepia()
        };
        let themes = themes.with(night).expect("a new name");
        assert_eq!(themes.resolve(None, Appearance::Dark), &dark());
    }

    #[test]
    fn a_choice_saved_under_the_old_names_resolves_to_the_same_theme() {
        let themes = Themes::built_in();
        assert_eq!(themes.resolve(Some("Dark"), Appearance::Light), &dark());
        assert_eq!(themes.resolve(Some("dark"), Appearance::Light), &dark());
        assert_eq!(themes.resolve(Some("Light"), Appearance::Dark), &light());
    }

    #[gpui_kit::test]
    fn the_applied_definition_is_the_light_one_until_one_is_set_and_then_that_one(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            assert_eq!(applied(cx), light());
            assert_eq!(palette(cx), light().palette);
            set_applied(sepia(), cx);
            assert_eq!(applied(cx), sepia());
            assert_eq!(palette(cx), sepia().palette);
            set_applied(dark(), cx);
            assert_eq!(applied(cx), dark());
        });
    }
}
