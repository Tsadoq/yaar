//! Component test (THEME-02): no source of the Frame, the three panes' crates or the Window
//! program writes a colour, or reads a colour of the toolkit's theme that a theme
//! definition does not hold, so that a third theme changes every colour on screen with no
//! change to the code that draws (NFR-019).
//!
//! The scan is the one that `yaar-navigator`'s test of the built-in table makes of its
//! sources: every `.rs` file of each `src/` folder is written out below and read with
//! `include_str!`, and a test holds the list to what is on disk, so that a new module is not
//! missed. The theme definitions (the Frame's `src/theme/`) are the one place a colour may
//! be written, and are left out.
//!
//! A line whose first characters are `//` is taken out: a doc comment may name a colour.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use yaar_frame::theme::light;

/// Each source file of the crates that draw, with its crate's folder: the Frame without
/// its `theme/` folder, the review pane, the terminal pane and the Window program.
macro_rules! sources {
    ($($folder:literal => [$($file:literal),+ $(,)?]),+ $(,)?) => {
        &[$($(($folder, $file, include_str!(concat!("../../", $folder, "/src/", $file)))),+),+]
    };
}

const SOURCES: &[(&str, &str, &str)] = sources!(
    "frame" => [
        "agents.rs",
        "consents.rs",
        "controls.rs",
        "keeper.rs",
        "lib.rs",
        "link_model.rs",
        "looks.rs",
        "models.rs",
        "navigation.rs",
        "routes.rs",
        "settings.rs",
        "settings_screen.rs",
        "sidebar.rs",
        "states.rs",
        "trace.rs",
        "view.rs",
    ],
    "review-pane" => [
        "actions.rs",
        "attaching.rs",
        "comments.rs",
        "draw.rs",
        "file_view.rs",
        "flow.rs",
        "flow_comment.rs",
        "flow_draw.rs",
        "follow.rs",
        "lib.rs",
        "live.rs",
        "live_draw.rs",
        "moved.rs",
        "names.rs",
        "navigate.rs",
        "rounds.rs",
        "saves.rs",
        "servers.rs",
        "shared.rs",
        "states.rs",
        "steady.rs",
        "threads.rs",
        "tree.rs",
        "turns.rs",
        "whose.rs",
    ],
    "terminal-pane" => [
        "attachment.rs",
        "consent.rs",
        "draw.rs",
        "emulator.rs",
        "keys.rs",
        "launchers.rs",
        "lib.rs",
        "modes.rs",
        "mouse.rs",
        "view.rs",
    ],
    "window" => [
        "appearance.rs",
        "lib.rs",
        "main.rs",
    ],
);

/// What writes a colour, each matched only where the character before it is no letter, digit
/// or `_`: `transparent_black()` is no `black()`, and `theme::hex(` is a `hex(`.
const COLOUR_WRITES: [&str; 17] = [
    "rgb(",
    "rgba(",
    "hsla(",
    "hsl(",
    "hex(",
    "hexa(",
    "Hsla {",
    "Rgba {",
    "Rgb {",
    "Rgb::new",
    "opaque_grey(",
    "black()",
    "white()",
    "red()",
    "green()",
    "blue()",
    "yellow()",
];

/// The fields of the toolkit's theme that are no colour, and may be read.
const NOT_COLOURS: [&str; 9] = [
    "mode",
    "font_family",
    "font_size",
    "mono_font_family",
    "mono_font_size",
    "radius",
    "radius_lg",
    "window_radius",
    "shadow",
];

/// Whether `c` can be part of a name.
fn is_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The lines of `source` that hold code, with their numbers from 1: those whose first
/// characters, after the blanks, are not `//`.
fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(at, line)| (at + 1, line))
}

/// Where `pattern` stands in `line` with no part of a name just before it.
fn found<'a>(line: &'a str, pattern: &'a str) -> impl Iterator<Item = usize> + 'a {
    line.match_indices(pattern)
        .map(|(at, _)| at)
        .filter(|&at| !line[..at].chars().next_back().is_some_and(is_name))
}

/// The colours `source` writes: the line and what is written there.
fn colours_written(source: &str) -> Vec<(usize, &'static str)> {
    let mut hits = Vec::new();
    for (number, line) in code_lines(source) {
        for pattern in COLOUR_WRITES {
            if found(line, pattern).next().is_some() {
                hits.push((number, pattern));
            }
        }
    }
    hits
}

/// The name read at the start of `text`: its letters, digits and `_`.
fn name_at(text: &str) -> &str {
    let end = text.find(|c| !is_name(c)).unwrap_or(text.len());
    &text[..end]
}

/// The names `source` reads of the toolkit's theme (`theme.<name>`, `theme().<name>`) that are
/// none of `roles` and no field that is no colour: the line and the name.
///
/// `theme.colors` as a whole is no read of a colour: it is the structure a definition is
/// written into, and the one place that names it is the model that applies a definition. A
/// role read through it, `theme.colors.<name>`, is judged as any other.
fn roles_unknown(source: &str, roles: &BTreeSet<&str>) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    for (number, line) in code_lines(source) {
        for prefix in ["theme().", "theme."] {
            for at in found(line, prefix) {
                let mut rest = &line[at + prefix.len()..];
                let mut name = name_at(rest);
                if name == "colors" && rest[name.len()..].starts_with('.') {
                    rest = &rest[name.len() + 1..];
                    name = name_at(rest);
                } else if name == "colors" {
                    continue;
                }
                if !name.is_empty() && !roles.contains(name) && !NOT_COLOURS.contains(&name) {
                    hits.push((number, name.to_string()));
                }
            }
        }
    }
    hits
}

/// The name of every role of a theme definition.
fn roles() -> BTreeSet<&'static str> {
    light()
        .colors
        .roles()
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// Every `.rs` file below `folder`, by its path from `folder` with `/` between parts.
fn rust_files(folder: &Path, below: &str, out: &mut Vec<String>) {
    for entry in fs::read_dir(folder.join(below)).expect("the source folder can be read") {
        let entry = entry.expect("an entry of the source folder");
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if below.is_empty() {
            name
        } else {
            format!("{below}/{name}")
        };
        if entry.file_type().expect("a file type").is_dir() {
            rust_files(folder, &path, out);
        } else if path.ends_with(".rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_list_of_sources_is_every_file_of_the_crates_that_draw_but_the_theme_definitions() {
    for folder in ["frame", "review-pane", "terminal-pane", "window"] {
        let mut on_disk = Vec::new();
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(folder)
            .join("src");
        rust_files(&src, "", &mut on_disk);
        // The one place a colour may be written.
        on_disk.retain(|path| !(folder == "frame" && path.starts_with("theme/")));
        on_disk.sort();
        let listed: Vec<&str> = SOURCES
            .iter()
            .filter(|(of, ..)| *of == folder)
            .map(|(_, file, _)| *file)
            .collect();
        assert_eq!(
            on_disk, listed,
            "a source file of {folder} is not in the list of this test, or is gone"
        );
    }
}

#[test]
fn no_source_of_the_frame_the_panes_or_the_window_program_writes_a_colour() {
    let hits: Vec<String> = SOURCES
        .iter()
        .flat_map(|(folder, file, source)| {
            colours_written(source)
                .into_iter()
                .map(move |(line, what)| format!("{folder}/src/{file}:{line}: writes `{what}`"))
        })
        .collect();
    assert!(
        hits.is_empty(),
        "a colour is written outside the theme definitions:\n{}",
        hits.join("\n")
    );
}

#[test]
fn no_source_of_the_frame_the_panes_or_the_window_program_reads_a_colour_no_definition_holds() {
    let roles = roles();
    let hits: Vec<String> = SOURCES
        .iter()
        .flat_map(|(folder, file, source)| {
            roles_unknown(source, &roles)
                .into_iter()
                .map(move |(line, name)| format!("{folder}/src/{file}:{line}: reads `{name}`"))
        })
        .collect();
    assert!(
        hits.is_empty(),
        "a role of the toolkit's theme is read that a definition does not hold:\n{}",
        hits.join("\n")
    );
}

// The control halves: the same scans find what they are meant to find, in text written here.

#[test]
fn the_scan_for_written_colours_finds_each_way_to_write_one() {
    let written = [
        ("div().bg(rgb(0x123456))", "rgb("),
        ("div().bg(theme::hex(0x123456))", "hex("),
        ("let c = rgba(0x12345678);", "rgba("),
        ("let c = hsla(0.1, 0.2, 0.3, 1.0);", "hsla("),
        ("let c = Hsla { h: 0.0, s: 0.0, l: 0.0, a: 1.0 };", "Hsla {"),
        ("let c = opaque_grey(0.5, 1.0);", "opaque_grey("),
        ("let c = Rgb::new(1, 2, 3);", "Rgb::new"),
        ("let c = Rgb { r, g, b };", "Rgb {"),
        ("div().text_color(white())", "white()"),
        ("div().text_color(red())", "red()"),
    ];
    for (text, pattern) in written {
        assert_eq!(
            colours_written(text),
            [(1, pattern)],
            "the scan of {text:?}"
        );
    }
    // A hit names its line, from 1.
    assert_eq!(
        colours_written("let a = 1;\nlet b = 2;\n    .bg(rgb(0x1))"),
        [(3, "rgb(")]
    );
}

#[test]
fn the_scan_for_written_colours_passes_what_draws_nothing_and_what_is_said_in_a_comment() {
    assert!(colours_written("div().bg(transparent_black())").is_empty());
    assert!(colours_written("div().bg(transparent_white())").is_empty());
    assert!(colours_written("let s = my_rgb(1);").is_empty());
    assert!(colours_written("let s = colour_hex(1);").is_empty());
    assert!(colours_written("    // theme::hex(0x1) and rgb(0x2)").is_empty());
    assert!(colours_written("/// It is `Hsla {` and `black()`.").is_empty());
    assert!(colours_written("let s = theme.accent;").is_empty());
}

#[test]
fn the_scan_for_roles_finds_a_name_no_definition_holds_and_passes_the_rest() {
    let roles = roles();
    assert!(
        !roles.is_empty(),
        "the roles of a definition are what the scan reads against"
    );
    let unknown = |text: &str| roles_unknown(text, &roles);

    assert_eq!(
        unknown("div().bg(theme.accent_hover)"),
        [(1, "accent_hover".to_string())]
    );
    assert_eq!(
        unknown("let a = 1;\nlet c = cx.theme().accent_hover;"),
        [(2, "accent_hover".to_string())]
    );
    assert_eq!(
        unknown("let c = theme.colors.accent_hover;"),
        [(1, "accent_hover".to_string())]
    );

    assert!(unknown("div().bg(theme.muted_foreground)").is_empty());
    assert!(unknown("div().bg(cx.theme().warning)").is_empty());
    assert!(unknown("let m = theme().mode;").is_empty());
    for field in NOT_COLOURS {
        assert!(
            unknown(&format!("let f = theme.{field};")).is_empty(),
            "{field}"
        );
    }
    assert!(unknown("Theme::update(cx, |theme| definition.onto(&mut theme.colors));").is_empty());
    assert!(unknown("let c = theme.colors.border;").is_empty());
    assert!(unknown("/// The theme.accent_hover is not read.").is_empty());
    assert!(unknown("let c = my_theme.accent_hover;").is_empty());
    assert!(unknown("\"the theme. Next\"").is_empty());
}
