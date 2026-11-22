//! Themes: every colour the Window draws, as one definition each (NFR-017 to NFR-019).
//!
//! A theme is one [`ThemeDefinition`]: a name, whether it is a light or a dark one
//! ([`Appearance`]), a colour for each role the Window draws with ([`Colors`]) and the
//! colours of a terminal ([`TerminalColors`]). Adding a theme to the app is one more
//! definition in this folder, listed in [`Themes::built_in`] and checked as [`Themes::with`]
//! checks one; nothing that draws changes, since nothing that draws names a colour: the
//! Frame and the panes read the toolkit's theme (`cx.theme()`), whose colours the applied
//! definition sets ([`ThemeDefinition::onto`]), and the terminal pane reads
//! [`TerminalColors`] of the applied definition ([`applied`], which `crate::Looks` sets).
//!
//! # The definitions the app is built with
//!
//! [`light`] and [`dark`], named [`LIGHT`] and [`DARK`]. Their colours are those the toolkit's
//! own light and dark themes give each role, so that the app looks as it did before slice 9
//! wherever the toolkit drew it: the package that writes them reads each role of
//! `cx.theme()` after `Theme::change(ThemeMode::Light)`, then `ThemeMode::Dark`, in a scratch
//! test of its own (not committed), and writes each as a literal with [`hex`] (or [`hexa`]
//! where its alpha is below 1), rounding each channel to the nearest of 0 to 255. Then, for
//! each pair of [`READABLE`], the text role must stand out from its ground by a contrast of
//! at least 4.5 to 1 (WCAG 2's contrast ratio of their relative luminances, alpha composited
//! over `background` first). Where a role falls short on a ground it is drawn on, it alone is
//! changed: moved away from the lightness of that ground, by 5 points of lightness in HSL a
//! step, to the first step that passes on every ground it is drawn on. Where no step that way
//! can pass, as for the light theme's `warning_foreground`, a near white text on a mid yellow,
//! the same steps are taken the other way. (The dark theme's `warning_foreground`, a dark text
//! on a light yellow, can only pass darker.) [`READABLE`] lists every pair of a text role and
//! a ground the Frame draws, among them the quiet text of the sidebar on its own ground, on
//! the selected row's, on the hovered row's and on `secondary`.
//!
//! The terminal's colours: `foreground` and `background` those of the theme, `cursor` the
//! theme's `foreground`, `selection` the theme's `selection`, and the sixteen of `ansi` in
//! xterm's order: 0 black `#000000`, 1 to 6 the theme's `red`, `green`, `yellow`, `blue`,
//! `magenta`, `cyan`, 7 white `#e5e5e5`, 8 bright black `#7f7f7f`, 9 to 14 the theme's
//! `red_light` to `cyan_light`, 15 bright white `#ffffff`: what the terminal pane drew before
//! slice 9. The dark theme's palette is that. The light theme's is that with each of 1 to 15
//! that reads under 3 to 1 on its white ground moved by the same steps, away from the ground's
//! lightness, to the first that reads 3 to 1 and is no colour of the palette before it (with
//! the dark theme's palette, white, bright white and yellow read 1.3 to 1.5 to 1 on white);
//! colour 0 is as it was. The roles `red` to `cyan_light` stay as they are: they are marks as
//! well as a palette.
//!
//! # What the definitions must hold
//!
//! [`Themes::with`] accepts a definition whose name is 1 to 64 characters, none a control
//! character, and differs from every name it holds, case aside; else [`ThemeError`]. The
//! unit tests of this module (THEME-03, proposed) hold each built-in definition to
//! [`READABLE`] and to [`DISTINCT`]: no two roles of a pair of [`DISTINCT`] are the same
//! colour, so that a selection, a hover or a mark never vanishes into its ground, and hold
//! the light theme's colours 1 to 15 of the terminal to 3 to 1 on its ground.
//!
//! # Which definition is applied
//!
//! [`Themes::resolve`]: the user's choice (`yaar_link::ThemeChoice`), or, with
//! `ThemeChoice::System` or a name no definition has, the built-in definition of the
//! system's appearance. The model `crate::Looks` applies it and keeps it applied.
//!
//! # Where colours may be written
//!
//! In this folder only. The terminal pane picks the colour of a cell in one public function
//! of its own (`yaar_terminal_pane::colour_of`): the definition's [`TerminalColors`] for the
//! default foreground and background and the sixteen numbered colours, [`indexed`] for the
//! numbers 16 to 255 and [`rgb24`] for a colour a program gives by value, which are xterm's,
//! not the theme's. The terminal also answers a program that asks for its colours (OSC 4,
//! 10, 11 and 12) from the applied definition's [`TerminalColors`].
//!
//! The test of colours in drawing code (THEME-02, proposed;
//! `crates/window/window/tests/colours.rs`) reads every source of the Frame, the three panes
//! and the Window program but this folder, and fails:
//! - on any colour written there: `rgb(`, `rgba(`, `hsla(`, `hsl(`, `hex(`, `hexa(`,
//!   `Hsla {`, `Rgba {`, `Rgb {`, `Rgb::new`, `opaque_grey(`, and the toolkit's named colours
//!   `black()`, `white()`, `red()`, `green()`, `blue()`, `yellow()`, each matched as a whole
//!   name (no letter, digit or `_` just before it), so that `transparent_black()` and
//!   `transparent_white()`, which draw nothing, stay allowed;
//! - on a role of the toolkit's theme read that is not one of [`Colors`]: every name read
//!   as `theme.<name>` or `theme().<name>` is a role of [`Colors::roles`] or one of the
//!   toolkit theme's fields that are no colour (`mode`, `font_family`, `font_size`,
//!   `mono_font_family`, `mono_font_size`, `radius`, `radius_lg`, `window_radius`,
//!   `shadow`). A role the definition does not hold would keep the toolkit's colour of the
//!   mode under any third theme.

mod dark;
mod light;

use gpui_kit::component::theme::ThemeColor;
use gpui_kit::*;

pub use dark::dark;
pub use light::light;

/// The name of the light theme the app is built with.
pub const LIGHT: &str = "Light";

/// The name of the dark theme the app is built with.
pub const DARK: &str = "Dark";

/// Whether a theme is a light or a dark one, and what the system's setting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    /// Dark text on a light ground. What the app shows where the system says nothing.
    #[default]
    Light,
    /// Light text on a dark ground.
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

/// A colour of `0xRRGGBB`, opaque.
pub fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

/// A colour of `0xRRGGBBAA`.
pub fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

/// The colour xterm gives the numbered colours 16 to 255 a program asks for: the 6 by 6 by 6
/// cube from 16 to 231 (each channel 0, or 55 plus 40 times its step), and 24 greys from 232
/// (8 plus 10 times its step). Under 16 the theme decides ([`TerminalColors::ansi`]), and
/// the terminal pane never asks this for those; it gives xterm's own sixteen there.
pub fn indexed(index: u8) -> Hsla {
    /// xterm's own sixteen, which the doc says this gives under 16.
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

/// The colour a program asks for by its value, red, green and blue.
pub fn rgb24(red: u8, green: u8, blue: u8) -> Hsla {
    hex(u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue))
}

/// Declares the roles of [`Colors`] once: the struct, the copy onto the toolkit's colours,
/// the copy from them and the list of the roles with their names.
macro_rules! roles {
    ($( $(#[$doc:meta])* $role:ident, )+) => {
        /// The colour of each role the Window draws with: the toolkit's roles the Frame,
        /// the panes and the toolkit's field of text read. Each is the toolkit's role of
        /// the same name.
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct Colors {
            $( $(#[$doc])* pub $role: Hsla, )+
        }

        impl Colors {
            /// Writes each role into the toolkit's colours of the same name; every other
            /// role of the toolkit stays as it is.
            pub fn onto(&self, kit: &mut ThemeColor) {
                $( kit.$role = self.$role; )+
            }

            /// The roles as the toolkit's colours hold them.
            pub fn of(kit: &ThemeColor) -> Colors {
                Colors { $( $role: kit.$role, )+ }
            }

            /// Every role with its name, in the order of the struct.
            pub fn roles(&self) -> Vec<(&'static str, Hsla)> {
                vec![ $( (stringify!($role), self.$role), )+ ]
            }
        }
    };
}

roles! {
    /// The ground of the window.
    background,
    /// Text.
    foreground,
    /// Lines between parts, and the frame of a control.
    border,
    /// The frame of a field of text.
    input,
    /// The ring of a control with the focus.
    ring,
    /// The caret of a field of text.
    caret,
    /// Selected text.
    selection,
    /// The colour of the main mark: the cursor's bar in a list, a chosen control.
    primary,
    /// Text on `primary`.
    primary_foreground,
    /// The ground of a selected row.
    secondary,
    /// Text on `secondary`.
    secondary_foreground,
    /// A quiet ground.
    muted,
    /// Quiet text: what cannot be chosen, a note.
    muted_foreground,
    /// The ground of a control under the pointer.
    accent,
    /// Text on `accent`.
    accent_foreground,
    /// A row under the pointer.
    list_hover,
    /// What was added, what went well.
    success,
    /// What waits, what to look at.
    warning,
    /// Text on `warning`.
    warning_foreground,
    /// What was removed, what failed.
    danger,
    /// A plain mark.
    info,
    /// The ground of the sidebar.
    sidebar,
    /// Text of the sidebar.
    sidebar_foreground,
    /// The line beside the sidebar.
    sidebar_border,
    /// The ground of the selected workspace in the sidebar.
    sidebar_accent,
    /// Its text.
    sidebar_accent_foreground,
    /// Red, as a mark and as a terminal's colour 1.
    red,
    /// Bright red, a terminal's colour 9.
    red_light,
    /// Green, colour 2.
    green,
    /// Bright green, colour 10.
    green_light,
    /// Yellow, colour 3.
    yellow,
    /// Bright yellow, colour 11.
    yellow_light,
    /// Blue, colour 4.
    blue,
    /// Bright blue, colour 12.
    blue_light,
    /// Magenta, colour 5.
    magenta,
    /// Bright magenta, colour 13.
    magenta_light,
    /// Cyan, colour 6.
    cyan,
    /// Bright cyan, colour 14.
    cyan_light,
}

/// The pairs of roles whose first is text drawn on the second, and must be readable there:
/// a contrast of at least 4.5 to 1. Every pair of a text role and a ground that the Frame
/// draws is listed: the sidebar draws quiet text on its own ground, on the selected row's and
/// on the hovered row's, and its plain text on the hovered row's.
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
];

/// The pairs of roles that must never be the same colour.
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

/// The colours of a terminal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalColors {
    /// The text a program writes with no colour of its own.
    pub foreground: Hsla,
    /// The ground of a cell with no colour of its own.
    pub background: Hsla,
    /// The cursor.
    pub cursor: Hsla,
    /// Selected cells.
    pub selection: Hsla,
    /// The sixteen colours a program asks for by the numbers 0 to 15, in xterm's order.
    pub ansi: [Hsla; 16],
}

/// One theme: everything the Window draws in it.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeDefinition {
    /// Its name, as the settings show it and as the user's choice keeps it.
    pub name: String,
    /// Whether it is light or dark: the toolkit's mode it is applied in, and the window's
    /// own appearance on macOS while it is chosen.
    pub appearance: Appearance,
    /// The colour of each role.
    pub colors: Colors,
    /// The colours of a terminal.
    pub terminal: TerminalColors,
}

impl ThemeDefinition {
    /// Writes its colours into the toolkit's: [`Colors::onto`].
    pub fn onto(&self, kit: &mut ThemeColor) {
        self.colors.onto(kit);
    }
}

/// Why a definition was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeError {
    /// Its name is empty, longer than 64 characters, or holds a control character.
    BadName(String),
    /// A definition of that name, case aside, is there already.
    Taken(String),
}

/// The definitions the Window can apply: the two built in, and those added.
#[derive(Debug, Clone, PartialEq)]
pub struct Themes {
    definitions: Vec<ThemeDefinition>,
}

impl Themes {
    /// The two definitions the app is built with, [`light`] then [`dark`].
    pub fn built_in() -> Themes {
        Themes {
            definitions: vec![light(), dark()],
        }
    }

    /// These themes and `definition` after them, when the module doc's rules accept it.
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

    /// Every definition, in the order they were added.
    pub fn all(&self) -> &[ThemeDefinition] {
        &self.definitions
    }

    /// The definition of this name, case aside.
    pub fn named(&self, name: &str) -> Option<&ThemeDefinition> {
        let name = name.to_lowercase();
        self.definitions
            .iter()
            .find(|definition| definition.name.to_lowercase() == name)
    }

    /// The definition the Window applies for the user's `choice` while the system's
    /// setting is `system`, as the module doc says.
    pub fn resolve(&self, choice: &yaar_link::ThemeChoice, system: Appearance) -> &ThemeDefinition {
        if let yaar_link::ThemeChoice::Named(name) = choice
            && let Some(definition) = self.named(name)
        {
            return definition;
        }
        // `built_in` put the light one first and the dark one second, and `with` only
        // adds after them, so these two places are the built-in definitions.
        match system {
            Appearance::Light => &self.definitions[0],
            Appearance::Dark => &self.definitions[1],
        }
    }
}

/// The definition applied, kept as a global of the app.
struct Applied(ThemeDefinition);

impl Global for Applied {}

/// The definition applied now, as `crate::Looks` last applied it: [`light`] before it ever
/// applied one. The terminal pane reads its [`TerminalColors`] here.
pub fn applied(cx: &App) -> ThemeDefinition {
    cx.try_global::<Applied>()
        .map_or_else(light, |applied| applied.0.clone())
}

/// Keeps `definition` as the one [`applied`] gives. `crate::Looks` calls this as the third
/// step of applying a definition; it writes nothing into the toolkit's colours. It is the
/// crate's own: nothing outside the Frame may set the applied definition around `Looks`.
// Nothing but a test calls it until `Looks` does its third step; whoever writes that call removes
// this attribute.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "`Looks` is the caller, not yet written")
)]
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

    /// A third theme, with every role and every terminal colour a value of its own.
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

    /// WCAG 2's relative luminance of an opaque colour.
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

    /// `colour` laid over the opaque `ground`.
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

    /// The contrast ratio of `text` on `ground`, each laid over `backdrop` first.
    fn contrast(text: Hsla, ground: Hsla, backdrop: Hsla) -> f32 {
        let (a, b) = (
            luminance(composited(text, backdrop)),
            luminance(composited(ground, backdrop)),
        );
        let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
        (lighter + 0.05) / (darker + 0.05)
    }

    /// `colour` with each channel rounded to the nearest of 0 to 255, as a literal of this
    /// folder writes it.
    fn rounded(colour: Hsla) -> Hsla {
        let [r, g, b, _] = channels(colour);
        hex(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }

    /// The rule that makes a colour readable: `start` moved away from the lightness of
    /// `ground` by 5 points of HSL lightness a step, to the first step (the colour itself
    /// being step 0) that `passes`; and where no step that way passes before the lightness
    /// runs out, the same steps the other way.
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
        // Chosen by a name written another way, it is still found.
        assert_eq!(
            themes.resolve(&ThemeChoice::Named("SEPIA".to_owned()), Appearance::Dark),
            &sepia()
        );

        // Applied: every role of the definition reaches the toolkit's colours, and no other.
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
        // Every kind of control character, not only the newline.
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
        // Characters, not bytes: 64 of two bytes each is a name.
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
        // The built-in themes by their own names, whatever the system says.
        assert_eq!(
            themes.resolve(&ThemeChoice::Named(DARK.to_owned()), Appearance::Light),
            &dark()
        );
        assert_eq!(
            themes.resolve(&ThemeChoice::Named(LIGHT.to_owned()), Appearance::Dark),
            &light()
        );
        // A third theme of a dark appearance is not what "as the system" gives.
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
        // Mid grey on white, as WCAG's own tables have it: 4.48 to 1 for #777777.
        assert!((contrast(hex(0x777777), white, white) - 4.48).abs() < 0.01);
        // A ground with alpha is laid over the backdrop first: half black on white is
        // mid grey, and white text on it reads 4.0 to 1.
        let half = hexa(0x00000080);
        assert!((contrast(white, half, white) - 4.0).abs() < 0.02);
        // The kit's own pair that the built-in light theme had to leave: white text on its
        // yellow.
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
                &["warning_foreground", "danger", "muted_foreground"][..],
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
                // A changed role is what the rule gives from the toolkit's colour: the
                // first step that reads 4.5 to 1 on every ground it is drawn on.
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

    /// The colour the terminal pane drew for each of the sixteen before slice 9: xterm's
    /// own black, the theme's six and their bright ones, xterm's white, bright black and
    /// bright white.
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
            // Moved by the rule from the colour drawn before, and still not another's.
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
    fn the_numbered_colours_above_the_sixteen_are_xterms_and_a_value_is_itself() {
        // Corners of the cube: 16 is black, 21 full blue, 196 full red, 231 white.
        assert_eq!(indexed(16), hex(0x000000));
        assert_eq!(indexed(21), hex(0x0000ff));
        assert_eq!(indexed(196), hex(0xff0000));
        assert_eq!(indexed(231), hex(0xffffff));
        // One step of each channel: 55, then 95 plus 40 for each step after it.
        assert_eq!(indexed(17), hex(0x00005f));
        assert_eq!(indexed(16 + 36), hex(0x5f0000));
        assert_eq!(indexed(16 + 6 * 2 + 3), hex(0x0087af));
        // The greys: 8 plus 10 a step, 232 to 255.
        assert_eq!(indexed(232), hex(0x080808));
        assert_eq!(indexed(244), hex(0x808080));
        assert_eq!(indexed(255), hex(0xeeeeee));
        // Under 16 it gives xterm's own sixteen, not the theme's.
        assert_eq!(indexed(1), hex(0xcd0000));
        assert_eq!(indexed(15), hex(0xffffff));
        assert_eq!(rgb24(0x12, 0x34, 0x56), hex(0x123456));
    }
}
