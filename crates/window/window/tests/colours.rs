//! No source of the Frame, the panes or the Window writes a colour or reads a theme role no
//! definition holds, but the theme definitions and the terminal pane's `colour_of`.
//! Not seen: a theme bound under another name (`let t = cx.theme(); t.role`).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use yaar_ui::themes::KIT_ROLES;

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

/// Matched only after no letter, digit or `_`: `transparent_black()` is no `black()`.
const COLOUR_WRITES: [&str; 19] = [
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
    "rgb24(",
    "indexed(",
];

const XTERM_COLOURS: [&str; 2] = ["rgb24(", "indexed("];

const XTERM_FILE: (&str, &str) = ("terminal-pane", "draw.rs");
const XTERM_FUNCTION: &str = "pub fn colour_of(";

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

fn is_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(at, line)| (at + 1, line))
}

fn found<'a>(line: &'a str, pattern: &'a str) -> impl Iterator<Item = usize> + 'a {
    line.match_indices(pattern)
        .map(|(at, _)| at)
        .filter(|&at| !line[..at].chars().next_back().is_some_and(is_name))
}

fn function_span(source: &str, opening: &str) -> Option<std::ops::RangeInclusive<usize>> {
    let first = source.lines().position(|line| line.contains(opening))? + 1;
    let last = source
        .lines()
        .enumerate()
        .skip(first)
        .find(|(_, line)| *line == "}")
        .map_or(usize::MAX, |(at, _)| at + 1);
    Some(first..=last)
}

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

fn colours_not_allowed(folder: &str, file: &str, source: &str) -> Vec<(usize, &'static str)> {
    let allowed = if (folder, file) == XTERM_FILE {
        function_span(source, XTERM_FUNCTION)
    } else {
        None
    };
    colours_written(source)
        .into_iter()
        .filter(|(number, pattern)| {
            !(XTERM_COLOURS.contains(pattern)
                && allowed.as_ref().is_some_and(|span| span.contains(number)))
        })
        .collect()
}

fn name_at(text: &str) -> &str {
    let end = text.find(|c| !is_name(c)).unwrap_or(text.len());
    &text[..end]
}

const WRITES_THE_KITS_COLOURS: (&str, &str) = ("frame", "looks.rs");

fn theme_reads(line: &str) -> Vec<(&str, &str)> {
    let mut reads = Vec::new();
    for prefix in ["theme().", "theme."] {
        for at in found(line, prefix) {
            reads.push((&line[..at], &line[at + prefix.len()..]));
        }
    }
    const GLOBAL: &str = "Theme::global(";
    for at in found(line, GLOBAL) {
        let rest = &line[at + GLOBAL.len()..];
        let tail = rest
            .find(')')
            .and_then(|close| rest[close + 1..].strip_prefix('.'));
        if let Some(tail) = tail {
            reads.push((&line[..at], tail));
        }
    }
    reads
}

/// Only in [`WRITES_THE_KITS_COLOURS`] is `&mut theme.colors` a write rather than a read;
/// elsewhere it is judged as the role name `colors`.
fn roles_unknown(
    folder: &str,
    file: &str,
    source: &str,
    roles: &BTreeSet<&str>,
) -> Vec<(usize, String)> {
    let may_write = (folder, file) == WRITES_THE_KITS_COLOURS;
    let mut hits = Vec::new();
    for (number, line) in code_lines(source) {
        for (before, mut rest) in theme_reads(line) {
            let mut name = name_at(rest);
            if name == "colors" && rest[name.len()..].starts_with('.') {
                rest = &rest[name.len() + 1..];
                name = name_at(rest);
            } else if name == "colors" && may_write && before.ends_with("&mut ") {
                continue;
            }
            if !name.is_empty() && !roles.contains(name) && !NOT_COLOURS.contains(&name) {
                hits.push((number, name.to_string()));
            }
        }
    }
    hits
}

fn roles() -> BTreeSet<&'static str> {
    KIT_ROLES.into_iter().collect()
}

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
fn the_list_of_sources_is_every_file_of_the_crates_that_draw() {
    for folder in ["frame", "review-pane", "terminal-pane", "window"] {
        let mut on_disk = Vec::new();
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(folder)
            .join("src");
        rust_files(&src, "", &mut on_disk);
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
            colours_not_allowed(folder, file, source)
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
            roles_unknown(folder, file, source, &roles)
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
        ("div().bg(crate::theme::rgb24(0xff, 0xaa, 0x00))", "rgb24("),
        ("div().bg(theme::indexed(196))", "indexed("),
    ];
    for (text, pattern) in written {
        assert_eq!(
            colours_written(text),
            [(1, pattern)],
            "the scan of {text:?}"
        );
    }
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
    assert!(colours_written("let s = my_indexed(1);").is_empty());
    assert!(colours_written("use yaar_frame::theme::{indexed, rgb24};").is_empty());
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
    let unknown = |text: &str| roles_unknown("frame", "view.rs", text, &roles);

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
    // The roles handed on whole are read by names this scan cannot see.
    assert_eq!(
        unknown("let c = &theme.colors;"),
        [(1, "colors".to_string())]
    );
    assert_eq!(
        unknown("let c = cx.theme().colors;\nc.button_hover"),
        [(1, "colors".to_string())]
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
    let write = "Theme::update(cx, |theme| definition.onto(&mut theme.colors));";
    assert!(roles_unknown("frame", "looks.rs", write, &roles).is_empty());
    for (folder, file) in [
        ("frame", "view.rs"),
        ("frame", "settings_screen.rs"),
        ("review-pane", "looks.rs"),
        ("terminal-pane", "draw.rs"),
        ("window", "appearance.rs"),
    ] {
        assert_eq!(
            roles_unknown(folder, file, write, &roles),
            [(1, "colors".to_string())],
            "{folder}/src/{file} may not write the kit's colours"
        );
    }
    assert!(unknown("let c = theme.colors.border;").is_empty());
    assert!(unknown("/// The theme.accent_hover is not read.").is_empty());
    assert!(unknown("let c = my_theme.accent_hover;").is_empty());
    assert!(unknown("\"the theme. Next\"").is_empty());
}

#[test]
fn the_scan_for_roles_reads_the_themes_global_as_it_reads_the_theme_of_a_context() {
    let roles = roles();
    let unknown = |text: &str| roles_unknown("review-pane", "draw.rs", text, &roles);

    assert_eq!(
        unknown("div().bg(Theme::global(cx).accent_hover)"),
        [(1, "accent_hover".to_string())]
    );
    assert_eq!(
        unknown("let a = 1;\nlet c = Theme::global(&*cx).accent_hover;"),
        [(2, "accent_hover".to_string())]
    );
    assert_eq!(
        unknown("let c = Theme::global(cx).colors.accent_hover;"),
        [(1, "accent_hover".to_string())]
    );
    assert_eq!(
        unknown("let c = &Theme::global(cx).colors;"),
        [(1, "colors".to_string())]
    );
    assert_eq!(
        unknown("(Theme::global(cx).danger, Theme::global(cx).nonsense)"),
        [(1, "nonsense".to_string())]
    );

    assert!(unknown("div().bg(Theme::global(cx).muted_foreground)").is_empty());
    assert!(unknown("let m = Theme::global(cx).mode;").is_empty());
    assert!(unknown("let c = Theme::global(cx).colors.border;").is_empty());
    assert!(unknown("let t = Theme::global(cx);").is_empty());
    assert!(unknown("let c = MyTheme::global(cx).accent_hover;").is_empty());
    assert!(unknown("// Theme::global(cx).accent_hover is not read").is_empty());
}

#[test]
fn xterm_colours_are_written_in_the_terminal_panes_colour_of_and_nowhere_else() {
    let (folder, file) = XTERM_FILE;
    let text = "\
pub fn colour_of(colour: Color) -> Colour {
    match colour {
        Color::Indexed(number) => indexed(number),
        Color::Spec(value) => rgb24(value.r, value.g, value.b),
    }
}

fn elsewhere() -> Colour {
    rgb24(1, 2, 3)
}
";
    assert_eq!(
        colours_not_allowed(folder, file, text),
        [(9, "rgb24(")],
        "only the line outside the function is found"
    );
    assert_eq!(colours_not_allowed(folder, "keys.rs", text).len(), 3);
    assert_eq!(colours_not_allowed("frame", file, text).len(), 3);
    assert_eq!(colours_not_allowed("window", "lib.rs", text).len(), 3);
    let other = "pub fn colour_of(c: Color) -> Colour {\n    rgb(0x123456)\n}\n";
    assert_eq!(colours_not_allowed(folder, file, other), [(2, "rgb(")]);
    assert_eq!(
        colours_not_allowed(folder, file, "fn x() { rgb24(1, 2, 3) }").len(),
        1
    );
}

#[test]
fn the_terminal_pane_really_writes_its_xterm_colours_in_the_function_the_exception_names() {
    // A rename of the function would make the scan pass for the wrong reason.
    let (folder, file) = XTERM_FILE;
    let (_, _, source) = SOURCES
        .iter()
        .find(|(of, name, _)| (*of, *name) == (folder, file))
        .expect("the file of the exception is in the list");
    let span = function_span(source, XTERM_FUNCTION).expect("the function of the exception");
    let mut inside: Vec<&str> = colours_written(source)
        .into_iter()
        .filter(|(number, _)| span.contains(number))
        .map(|(_, pattern)| pattern)
        .collect();
    inside.sort_unstable();
    assert_eq!(
        inside,
        ["indexed(", "rgb24("],
        "the function writes both, once"
    );
    assert!(colours_not_allowed(folder, file, source).is_empty());
}
