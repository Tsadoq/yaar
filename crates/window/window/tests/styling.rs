//! No source of the Frame, the panes or the Window program writes a colour or reads a theme role
//! no definition holds, but the theme definitions and the terminal pane's `colour_of`; a crate in
//! `STRICT` also leaves fonts, sizes, corners, shadows and the pointer to `yaar-ui`.

#[path = "styling/scan.rs"]
mod scan;

use std::fs;
use std::path::{Path, PathBuf};

use scan::{
    CRATES, Finding, NOT_COLOURS, Rule, Source, THEME_GLOBAL, THEME_OF_A_CONTEXT,
    THEME_OF_ANYTHING, THEME_TRAIT, XTERM_FILE, XTERM_FUNCTION, code_of, colours_not_allowed,
    colours_written, crates_folder, function_span, roles, roles_unknown, scan, sources_under,
};

/// The crates held to the full styling rule, not only to the colour rules.
const STRICT: &[&str] = &["frame", "review-pane"];

fn said(findings: &[Finding], rule: Rule) -> Vec<String> {
    findings
        .iter()
        .filter(|finding| finding.rule == rule)
        .map(Finding::said)
        .collect()
}

fn of_the_repository(rule: Rule) -> Vec<String> {
    let sources = sources_under(&crates_folder());
    for krate in CRATES {
        assert!(
            sources.iter().any(|source| source.krate == krate),
            "no source of {krate} was found: the walk looks in the wrong place"
        );
    }
    said(&scan(&sources, STRICT), rule)
}

fn in_crate(krate: &str, text: &str) -> Source {
    Source {
        krate: krate.to_string(),
        file: "view.rs".to_string(),
        text: text.to_string(),
    }
}

fn scanned(krate: &str, strict: &[&str], text: &str) -> Vec<(usize, Rule, String)> {
    scan(&[in_crate(krate, text)], strict)
        .into_iter()
        .map(|finding| (finding.line, finding.rule, finding.what))
        .collect()
}

struct TemporaryFolder(PathBuf);

impl TemporaryFolder {
    fn new(name: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("yaar-styling-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("a temporary folder");
        TemporaryFolder(folder)
    }
}

impl Drop for TemporaryFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a folder to copy into");
    for entry in fs::read_dir(from).expect("a folder to copy") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("a file copied");
        }
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a folder")).expect("a folder to write into");
    fs::write(path, text).expect("a file written");
}

#[test]
fn a_new_source_file_is_scanned_without_being_listed() {
    let copy = TemporaryFolder::new("walk");
    copy_tree(
        &crates_folder().join("frame/src"),
        &copy.0.join("frame/src"),
    );
    let colour = "fn paint() { let _ = rgb(0x123456); }\n";
    write(&copy.0.join("frame/src/brand_new.rs"), colour);
    write(&copy.0.join("frame/src/deeper/still/new.rs"), colour);
    write(&copy.0.join("frame/src/not_rust.txt"), colour);
    write(&copy.0.join("review-pane/src/lib.rs"), colour);
    write(&copy.0.join("ui/src/colours.rs"), colour);

    let sources = sources_under(&copy.0);
    let files: Vec<String> = sources
        .iter()
        .map(|source| format!("{}/{}", source.krate, source.file))
        .collect();
    for wanted in [
        "frame/lib.rs",
        "frame/brand_new.rs",
        "frame/deeper/still/new.rs",
        "review-pane/lib.rs",
    ] {
        assert!(files.contains(&wanted.to_string()), "{wanted} in {files:?}");
    }
    assert!(!files.iter().any(|file| file.contains("not_rust")));
    assert!(!files.iter().any(|file| file.starts_with("ui/")));

    let mut written: Vec<String> = said(&scan(&sources, &[]), Rule::Colour);
    written.sort();
    assert_eq!(
        written,
        [
            "frame/src/brand_new.rs:1: writes `rgb(`",
            "frame/src/deeper/still/new.rs:1: writes `rgb(`",
            "review-pane/src/lib.rs:1: writes `rgb(`",
        ],
        "the sources of the repository were clean, so only the new files are found"
    );
}

#[test]
fn in_a_strict_crate_each_styling_call_is_found() {
    let calls = [
        ("div().font_family(\"Inter\")", "font_family"),
        ("div().text_size(px(13.))", "text_size"),
        ("div().text_xs()", "text_xs"),
        ("div().text_sm()", "text_sm"),
        ("div().text_base()", "text_base"),
        ("div().text_lg()", "text_lg"),
        ("div().text_xl()", "text_xl"),
        ("div().text_2xl()", "text_2xl"),
        ("div().text_3xl()", "text_3xl"),
        ("div().font_weight(weight)", "font_weight"),
        ("div().rounded_md()", "rounded"),
        ("div().rounded_full()", "rounded"),
        ("div().rounded(px(3.))", "rounded"),
        ("div().shadow_lg()", "shadow"),
        ("div().shadow(vec![])", "shadow"),
        ("div().border_dashed()", "border_dashed"),
        ("div().cursor_pointer()", "cursor_pointer"),
        ("let c = cx.theme().accent;", THEME_OF_A_CONTEXT),
        ("let c = window.theme().accent;", THEME_OF_ANYTHING),
        ("let c = self.app().theme().accent;", THEME_OF_ANYTHING),
        ("use gpui_kit::ActiveTheme;", THEME_TRAIT),
        ("impl ActiveTheme for Thing {}", THEME_TRAIT),
        ("let c = Theme::global(cx).accent;", THEME_GLOBAL),
        ("let t = Theme::global(&*cx);", THEME_GLOBAL),
        ("let f = style::font_weight(1);", "font_weight"),
        ("let c = theme.danger.opacity(0.16);", "opacity"),
        ("div().opacity(0.5)", "opacity"),
        ("run.weight = FontWeight::BOLD;", "FontWeight"),
        ("let w = gpui_kit::FontWeight::NORMAL;", "FontWeight"),
        ("let w = FontWeight(400.);", "FontWeight"),
        ("let f = font(family.clone());", "font"),
        ("let f = (font (family));", "font"),
        ("div().font(f)", "font"),
        ("div().cursor(CursorStyle::IBeam)", "cursor"),
        ("div().cursor( style )", "cursor"),
    ];
    for (text, name) in calls {
        for krate in CRATES {
            assert_eq!(
                scanned(krate, &[krate], text),
                [(1, Rule::Styling, name.to_string())],
                "the scan of {text:?} in {krate}"
            );
        }
    }

    let text: String = calls.iter().map(|(text, _)| format!("{text}\n")).collect();
    let found: Vec<&str> = scanned("frame", &["frame"], &text)
        .iter()
        .map(|(line, ..)| *line)
        .map(|line| calls[line - 1].1)
        .collect();
    assert_eq!(
        found,
        calls.map(|(_, name)| name),
        "each call once, in order"
    );

    assert_eq!(
        scanned(
            "frame",
            &["frame"],
            "div().text_sm().text_sm().rounded_md().rounded_lg()"
        ),
        [
            (1, Rule::Styling, "text_sm".to_string()),
            (1, Rule::Styling, "rounded".to_string()),
        ],
        "a name is said once a line"
    );
    assert_eq!(
        scanned(
            "frame",
            &["frame"],
            "div()\n    .rounded_md()\n    .child(x)\n    .shadow_sm()"
        ),
        [
            (2, Rule::Styling, "rounded".to_string()),
            (4, Rule::Styling, "shadow".to_string()),
        ],
        "a call on its own line is found where it is"
    );
    let other = &["review-pane", "window"];
    assert!(
        scanned("frame", other, "div().text_sm().rounded_md()").is_empty(),
        "the strictness is that of the crate the file is in"
    );
}

#[test]
fn in_a_crate_not_yet_strict_only_colour_writes_are_found() {
    let text = "\
div().text_sm().rounded_md().shadow_lg().cursor_pointer().font_weight(w).border_dashed()
let c = cx.theme().accent;
use gpui_kit::ActiveTheme;
div().bg(rgb(0x123456))
";
    for krate in CRATES {
        assert_eq!(
            scanned(krate, &[], text),
            [(4, Rule::Colour, "rgb(".to_string())],
            "{krate}"
        );
    }
    let strict = scanned("window", &["window"], text);
    assert_eq!(strict.len(), 9, "{strict:?}");
    assert!(strict.contains(&(4, Rule::Colour, "rgb(".to_string())));
    assert_eq!(
        scanned("review-pane", &["frame", "window"], text),
        [(4, Rule::Colour, "rgb(".to_string())],
        "a crate outside the strict set is not held to the styling rules"
    );
}

#[test]
fn a_styling_word_inside_a_comment_or_a_string_is_passed() {
    let passed = [
        "// div().rounded_md().text_sm() and cx.theme()",
        "/// Draws `.shadow_lg()` through `ActiveTheme`.",
        "//! The `.cursor_pointer()` of a row.",
        "let s = \"div().rounded_md().text_sm()\";",
        "let s = \"a \\\" quote, then .font_weight(1)\";",
        "let s = r#\"a \" quote, then .shadow_lg() and cx.theme()\"#;",
        "let s = br\"ActiveTheme\";",
        "div().child(\"cx.theme()\") /* .border_dashed() */",
        "/* .text_xl()\n   .rounded_sm()\n   ActiveTheme */",
        "/* outer /* .text_lg() nested */ .text_lg() still a comment */",
        "let line = \"first\\\n    .text_sm()\";",
        "let a = '\"'; // .text_sm()",
        "let s = \"//\"; /* .rounded_md() */",
    ];
    for text in passed {
        assert!(
            scanned("frame", &["frame"], text).is_empty(),
            "the scan of {text:?}"
        );
    }

    let unrelated = [
        "let n = self.surrounded();",
        "let n = x.shadowed_name();",
        "let n = x.text_small();",
        "let n = x.text_size_of(1);",
        "let n = x.font_weights;",
        "let n = x.my_theme(1);",
        "let n = x.theme;",
        "let n = x.theme_name();",
        "let n = x.opacity_of(c);",
        "let n = x.opacity;",
        "let n = my_opacity(c);",
        "let n = x.my_font(f);",
        "let n = x.font();",
        "let n = my_font(f);",
        "let n = font_of(f);",
        "let n = font;",
        "fn font(f: Family) {}",
        "let n = MyFontWeight::BOLD;",
        "let n = FontWeights::BOLD;",
        "let n = FontWeight;",
        "let n = self.flow.cursor();",
        "let n = self.cursor ( );",
        "let n = x.cursor_of(1);",
        "let n = x.cursors(1);",
        "let n = cx.themes();",
        "let n = abcx.theme_of();",
        "let rounded = 1; let shadow = 2; let font_family = 3;",
        "fn rounded(x: u32) {}",
        "use gpui_kit::ActiveThemeKind;",
        "let n = MyActiveTheme;",
        "let n = MyTheme::global(cx);",
        "let n = Theme::update(cx, f);",
        "let n = Theme::globals(cx);",
        "div().text_color(c).font_style(s).border_1()",
    ];
    for text in unrelated {
        assert!(
            scanned("frame", &["frame"], text).is_empty(),
            "the scan of {text:?}"
        );
    }

    let after = "let a = '\"'; x.text_sm();\nfn f<'a>(x: &'a str) { x.rounded_md(); }\nlet b = '\\''; y.shadow_sm();";
    assert_eq!(
        scanned("frame", &["frame"], after),
        [
            (1, Rule::Styling, "text_sm".to_string()),
            (2, Rule::Styling, "rounded".to_string()),
            (3, Rule::Styling, "shadow".to_string()),
        ],
        "a quote in a character or a lifetime opens no literal"
    );
    for text in [
        "let q = ['\\'','\"'];\ndiv().bg(rgb(1))\nlet s = \"x\";",
        "let q = matches!(c, '\\''|'\"');\ndiv().bg(rgb(1))\nlet s = \"x\";",
        "let q = ['\\\\','\\u{27}','\"'];\ndiv().bg(rgb(1))\nlet s = \"x\";",
    ] {
        assert_eq!(
            colours_written(text),
            [(2, "rgb(")],
            "an escaped quote in {text:?}"
        );
    }
    let lines = "a\n/* x\n y */ b\n\"c\nd\" e\nrgb(1)";
    assert_eq!(code_of(lines).lines().count(), lines.lines().count());
    assert_eq!(
        colours_written(lines),
        [(6, "rgb(")],
        "the line numbers hold"
    );
}

#[test]
fn no_source_of_the_frame_the_panes_or_the_window_program_writes_a_colour() {
    let hits = of_the_repository(Rule::Colour);
    assert!(
        hits.is_empty(),
        "a colour is written outside the theme definitions:\n{}",
        hits.join("\n")
    );
}

#[test]
fn no_source_of_the_frame_the_panes_or_the_window_program_reads_a_colour_no_definition_holds() {
    let hits = of_the_repository(Rule::Role);
    assert!(
        hits.is_empty(),
        "a role of the toolkit's theme is read that a definition does not hold:\n{}",
        hits.join("\n")
    );
}

#[test]
fn no_strict_crate_styles_what_the_ui_crate_paints() {
    for krate in STRICT {
        assert!(CRATES.contains(krate), "{krate} is no crate that draws");
    }
    let hits = of_the_repository(Rule::Styling);
    assert!(
        hits.is_empty(),
        "a strict crate styles itself instead of using the pieces of `yaar-ui`:\n{}",
        hits.join("\n")
    );
}

#[test]
fn every_crate_that_draws_is_scanned() {
    let folder = crates_folder();
    let mut unlisted = Vec::new();
    for entry in fs::read_dir(&folder).expect("the crates folder") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(manifest) = fs::read_to_string(path.join("Cargo.toml")) else {
            continue;
        };
        let uses_the_toolkit = manifest.lines().any(|line| {
            line.trim_start()
                .strip_prefix("gpui-kit")
                .is_some_and(|rest| {
                    rest.starts_with(|c: char| c.is_whitespace() || c == '=' || c == '.')
                })
        });
        if uses_the_toolkit && name != "ui" && !CRATES.contains(&name.as_str()) {
            unlisted.push(name);
        }
    }
    assert!(
        unlisted.is_empty(),
        "a crate draws with the toolkit and is not in the scan's list: {unlisted:?}"
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
    assert!(colours_written("let s = \"rgb(1) and black()\";").is_empty());
    assert!(colours_written("let s = 1; /* rgb(1) */").is_empty());
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
    let sources = sources_under(&crates_folder());
    let source = sources
        .iter()
        .find(|source| (source.krate.as_str(), source.file.as_str()) == (folder, file))
        .map(|source| source.text.as_str())
        .expect("the file of the exception is in the walk");
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
