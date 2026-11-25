use gpui_kit::Hsla;
use toml::{Table, Value};

use super::{Appearance, ThemeDefinition, valid_name};
use crate::colour::{hex, hexa};
use crate::tokens::{Palette, TOKEN_NAMES, TerminalColors};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeFileError {
    Toml(String),
    /// A key the file lacks: a token, or `name`, `appearance`, `colors`, `terminal` and the
    /// `terminal.` keys.
    Missing(String),
    /// A key the file holds that no token or setting has.
    Unknown(String),
    BadColour {
        token: String,
        value: String,
    },
    BadAppearance(String),
    BadName(String),
}

const TERMINAL_KEYS: [&str; 5] = ["foreground", "background", "cursor", "selection", "ansi"];

impl ThemeDefinition {
    pub fn from_toml(source: &str) -> Result<ThemeDefinition, ThemeFileError> {
        let mut file = toml::from_str::<Table>(source)
            .map_err(|error| ThemeFileError::Toml(error.to_string()))?;

        let name = match file.remove("name") {
            None => return Err(ThemeFileError::Missing("name".to_owned())),
            Some(Value::String(name)) if valid_name(&name) => name,
            Some(Value::String(name)) => return Err(ThemeFileError::BadName(name)),
            Some(other) => return Err(ThemeFileError::BadName(other.to_string())),
        };
        let appearance = match file.remove("appearance") {
            None => return Err(ThemeFileError::Missing("appearance".to_owned())),
            Some(Value::String(value)) if value == "light" => Appearance::Light,
            Some(Value::String(value)) if value == "dark" => Appearance::Dark,
            Some(Value::String(value)) => return Err(ThemeFileError::BadAppearance(value)),
            Some(other) => return Err(ThemeFileError::BadAppearance(other.to_string())),
        };
        let mut colors = table(&mut file, "colors")?;
        let mut terminal = table(&mut file, "terminal")?;
        if let Some(key) = file.keys().next() {
            return Err(ThemeFileError::Unknown(key.clone()));
        }

        if let Some(key) = colors
            .keys()
            .find(|key| !TOKEN_NAMES.contains(&key.as_str()))
        {
            return Err(ThemeFileError::Unknown(key.clone()));
        }
        let palette = Palette::build(|token| {
            let value = colors
                .remove(token)
                .ok_or_else(|| ThemeFileError::Missing(token.to_owned()))?;
            colour(token, &value)
        })?;

        if let Some(key) = terminal
            .keys()
            .find(|key| !TERMINAL_KEYS.contains(&key.as_str()))
        {
            return Err(ThemeFileError::Unknown(format!("terminal.{key}")));
        }
        let mut single = |key: &str| {
            let token = format!("terminal.{key}");
            let value = terminal
                .remove(key)
                .ok_or_else(|| ThemeFileError::Missing(token.clone()))?;
            colour(&token, &value)
        };
        let (foreground, background) = (single("foreground")?, single("background")?);
        let (cursor, selection) = (single("cursor")?, single("selection")?);
        let ansi = match terminal.remove("ansi") {
            None => return Err(ThemeFileError::Missing("terminal.ansi".to_owned())),
            Some(Value::Array(items)) => items,
            Some(_) => return structural("`terminal.ansi` is not a list of colours".to_owned()),
        };
        let ansi = ansi
            .iter()
            .enumerate()
            .map(|(index, value)| colour(&format!("terminal.ansi[{index}]"), value))
            .collect::<Result<Vec<_>, _>>()?;
        let ansi: [Hsla; 16] = ansi.try_into().or_else(|wrong: Vec<Hsla>| {
            structural(format!(
                "`terminal.ansi` holds {} colours, not sixteen",
                wrong.len()
            ))
        })?;

        Ok(ThemeDefinition {
            name,
            appearance,
            palette,
            terminal: TerminalColors {
                foreground,
                background,
                cursor,
                selection,
                ansi,
            },
        })
    }
}

fn structural<T>(message: String) -> Result<T, ThemeFileError> {
    Err(ThemeFileError::Toml(message))
}

fn table(file: &mut Table, key: &str) -> Result<Table, ThemeFileError> {
    match file.remove(key) {
        Some(Value::Table(table)) => Ok(table),
        Some(_) => structural(format!("`{key}` is not a table")),
        None => Err(ThemeFileError::Missing(key.to_owned())),
    }
}

/// `#RRGGBB`, or `#RRGGBBAA` for a translucent one.
fn colour(token: &str, value: &Value) -> Result<Hsla, ThemeFileError> {
    let bad = || ThemeFileError::BadColour {
        token: token.to_owned(),
        value: match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        },
    };
    let Value::String(text) = value else {
        return Err(bad());
    };
    let digits = text.strip_prefix('#').ok_or_else(bad)?;
    if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let number = u32::from_str_radix(digits, 16).map_err(|_| bad())?;
    match digits.len() {
        6 => Ok(hex(number)),
        8 => Ok(hexa(number)),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeFileError;
    use crate::colour::{hex, hexa};
    use crate::themes::{Appearance, ThemeDefinition};
    use crate::tokens::TOKEN_NAMES;

    const ANSI: &str = r##"["#000000", "#111111", "#222222", "#333333", "#444444", "#555555", "#666666", "#777777", "#888888", "#999999", "#aaaaaa", "#bbbbbb", "#cccccc", "#dddddd", "#eeeeee", "#ffffff"]"##;

    /// A whole file, its colours given by `colour_of`, and a line of `extra` under `[colors]`;
    /// a token `colour_of` gives nothing for is left out.
    fn file(colour_of: impl Fn(&str) -> Option<String>, extra: &str, ansi: &str) -> String {
        let colors: String = TOKEN_NAMES
            .iter()
            .filter_map(|name| colour_of(name).map(|colour| format!("{name} = \"{colour}\"\n")))
            .collect();
        format!(
            "name = \"Test\"\nappearance = \"light\"\n[colors]\n{colors}{extra}\n\
             [terminal]\nforeground = \"#fafafa\"\nbackground = \"#171717\"\n\
             cursor = \"#fafafa\"\nselection = \"#1447e666\"\nansi = {ansi}\n"
        )
    }

    fn plain(name: &str) -> Option<String> {
        Some(match name {
            "overlay" => "#12345678".to_owned(),
            "window" => "#abcdef".to_owned(),
            _ => "#101010".to_owned(),
        })
    }

    #[test]
    fn a_theme_file_with_every_token_parses_to_its_values() {
        let theme = ThemeDefinition::from_toml(&file(plain, "", ANSI)).expect("a whole file");
        assert_eq!(theme.name, "Test");
        assert_eq!(theme.appearance, Appearance::Light);
        assert_eq!(theme.palette.overlay, hexa(0x12345678));
        assert_eq!(theme.palette.window, hex(0xabcdef));
        assert_eq!(theme.palette.del_word, hex(0x101010));
        assert_eq!(theme.terminal.selection, hexa(0x1447e666));
        assert_eq!(theme.terminal.ansi[1], hex(0x111111));
        assert_eq!(theme.terminal.ansi[15], hex(0xffffff));
    }

    #[test]
    fn a_theme_file_missing_a_token_names_it() {
        let text = file(|name| plain(name).filter(|_| name != "del_word"), "", ANSI);
        assert_eq!(
            ThemeDefinition::from_toml(&text),
            Err(ThemeFileError::Missing("del_word".to_owned()))
        );
    }

    #[test]
    fn a_theme_file_with_a_key_no_token_has_names_it() {
        let text = file(plain, "shadow = \"#000000\"", ANSI);
        assert_eq!(
            ThemeDefinition::from_toml(&text),
            Err(ThemeFileError::Unknown("shadow".to_owned()))
        );
    }

    #[test]
    fn a_colour_not_written_as_six_or_eight_hex_digits_is_refused() {
        for bad in [
            "#fff", "red", "#12345", "#12345g", "+12345", "#+12345", "123456",
        ] {
            let text = file(
                |name| plain(name).map(|good| if name == "pane" { bad.to_owned() } else { good }),
                "",
                ANSI,
            );
            assert_eq!(
                ThemeDefinition::from_toml(&text),
                Err(ThemeFileError::BadColour {
                    token: "pane".to_owned(),
                    value: bad.to_owned()
                }),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_terminal_without_sixteen_colours_is_refused() {
        let fifteen = ANSI.replace(", \"#ffffff\"", "");
        assert_eq!(
            ThemeDefinition::from_toml(&file(plain, "", &fifteen)),
            Err(ThemeFileError::Toml(
                "`terminal.ansi` holds 15 colours, not sixteen".to_owned()
            ))
        );
        let seventeen = ANSI.replace("]", ", \"#000000\"]");
        assert_eq!(
            ThemeDefinition::from_toml(&file(plain, "", &seventeen)),
            Err(ThemeFileError::Toml(
                "`terminal.ansi` holds 17 colours, not sixteen".to_owned()
            ))
        );
        assert_eq!(
            ThemeDefinition::from_toml(&file(plain, "", "\"#000000\"")),
            Err(ThemeFileError::Toml(
                "`terminal.ansi` is not a list of colours".to_owned()
            ))
        );
    }

    #[test]
    fn a_colours_or_terminal_key_that_is_no_table_is_named_as_such() {
        let whole = file(plain, "", ANSI);
        let head = "name = \"Test\"\nappearance = \"light\"\n";
        let (colors, terminal) = whole
            .strip_prefix(head)
            .and_then(|body| body.split_once("[terminal]\n"))
            .expect("a whole file");
        for (key, text) in [
            (
                "colors",
                format!("{head}colors = 3\n[terminal]\n{terminal}"),
            ),
            ("terminal", format!("{head}terminal = 3\n{colors}")),
        ] {
            assert_eq!(
                ThemeDefinition::from_toml(&text),
                Err(ThemeFileError::Toml(format!("`{key}` is not a table")))
            );
        }
    }

    #[test]
    fn a_file_that_is_not_toml_or_has_no_usable_head_is_refused() {
        assert!(matches!(
            ThemeDefinition::from_toml("name = "),
            Err(ThemeFileError::Toml(_))
        ));
        let text = file(plain, "", ANSI);
        assert_eq!(
            ThemeDefinition::from_toml(
                &text.replace("appearance = \"light\"", "appearance = \"sepia\"")
            ),
            Err(ThemeFileError::BadAppearance("sepia".to_owned()))
        );
        assert_eq!(
            ThemeDefinition::from_toml(&text.replace("name = \"Test\"", "name = \"\"")),
            Err(ThemeFileError::BadName(String::new()))
        );
    }
}
