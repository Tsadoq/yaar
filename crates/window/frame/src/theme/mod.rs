//! Themes: every colour the Window draws, as one definition each (NFR-017 to NFR-019).
//!
//! A theme is one [`ThemeDefinition`]: a name, whether it is a light or a dark one
//! ([`Appearance`]), a colour for each role the Window draws with ([`Colors`]) and the
//! colours of a terminal ([`TerminalColors`]). Adding a theme to the app is one more
//! definition in this folder, listed in [`Themes::built_in`] and checked as [`Themes::with`]
//! checks one; nothing that draws changes, since nothing that draws names a colour: the
//! Frame and the panes read the toolkit's theme (`cx.theme()`), whose colours the applied
//! definition sets ([`ThemeDefinition::onto`]), and the terminal pane reads
//! [`TerminalColors`] of the applied definition ([`applied`]).
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
//! over `background` first); where a pair falls short, the text role alone is changed, in
//! steps of 5 points of lightness in HSL (darker on a light theme, lighter on a dark one),
//! until it passes. The terminal's colours: `foreground` and `background` those of the
//! theme, `cursor` the theme's `foreground`, `selection` the theme's `selection`, and the
//! sixteen of `ansi` in xterm's order: 0 black `#000000`, 1 to 6 the theme's `red`,
//! `green`, `yellow`, `blue`, `magenta`, `cyan`, 7 white `#e5e5e5`, 8 bright black
//! `#7f7f7f`, 9 to 14 the theme's `red_light` to `cyan_light`, 15 bright white `#ffffff`:
//! what the terminal pane drew before slice 9.
//!
//! # What the definitions must hold
//!
//! [`Themes::with`] accepts a definition whose name is 1 to 64 characters, none a control
//! character, and differs from every name it holds, case aside; else [`ThemeError`]. The
//! unit tests of this module (THEME-03, proposed) hold each built-in definition to
//! [`READABLE`] and to [`DISTINCT`]: no two roles of a pair of [`DISTINCT`] are the same
//! colour, so that a selection, a hover or a mark never vanishes into its ground.
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
    let _ = index;
    todo!("xterm's numbered colours, as the doc says")
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
/// a contrast of at least 4.5 to 1.
pub const READABLE: &[(&str, &str)] = &[
    ("foreground", "background"),
    ("muted_foreground", "background"),
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
    pub fn with(self, definition: ThemeDefinition) -> Result<Themes, ThemeError> {
        let _ = definition;
        todo!("check the name, then add, as the module doc says")
    }

    /// Every definition, in the order they were added.
    pub fn all(&self) -> &[ThemeDefinition] {
        &self.definitions
    }

    /// The definition of this name, case aside.
    pub fn named(&self, name: &str) -> Option<&ThemeDefinition> {
        let _ = name;
        todo!("find by name, case aside")
    }

    /// The definition the Window applies for the user's `choice` while the system's
    /// setting is `system`, as the module doc says.
    pub fn resolve(&self, choice: &yaar_link::ThemeChoice, system: Appearance) -> &ThemeDefinition {
        let _ = (choice, system);
        todo!("the chosen definition, or the built-in one of the system's appearance")
    }
}

/// The definition applied now, as `crate::Looks` last applied it: [`light`] before it ever
/// applied one. The terminal pane reads its [`TerminalColors`] here.
pub fn applied(cx: &App) -> ThemeDefinition {
    let _ = cx;
    todo!("the applied definition, kept as a global of the app by Looks")
}
