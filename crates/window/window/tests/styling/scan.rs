//! The scan: reads the sources of the crates that draw and says which line styles itself.
//! Not seen: a theme bound under another name (`let t = cx.theme(); t.role`).

use std::collections::BTreeSet;
use std::fs;
use std::ops::RangeInclusive;
use std::path::Path;

use yaar_ui::themes::KIT_ROLES;

/// The crates that draw, by folder; the walk reads the `src/` of each and never that of `ui`.
pub const CRATES: [&str; 4] = ["frame", "review-pane", "terminal-pane", "window"];

/// Matched only after no letter, digit or `_`: `transparent_black()` is no `black()`.
pub const COLOUR_WRITES: [&str; 19] = [
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

pub const XTERM_COLOURS: [&str; 2] = ["rgb24(", "indexed("];

pub const XTERM_FILE: (&str, &str) = ("terminal-pane", "draw.rs");
pub const XTERM_FUNCTION: &str = "pub fn colour_of(";

pub const NOT_COLOURS: [&str; 9] = [
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

const WRITES_THE_KITS_COLOURS: (&str, &str) = ("frame", "looks.rs");

/// Banned as `.name` or `::name`, whole.
const STYLING_NAMES: [&str; 12] = [
    "font_family",
    "text_size",
    "text_xs",
    "text_sm",
    "text_base",
    "text_lg",
    "text_xl",
    "text_2xl",
    "text_3xl",
    "font_weight",
    "border_dashed",
    "cursor_pointer",
];

/// Banned as a method call `.name(..)`. A zero-argument `cursor()` or `font()` is a reader, not
/// a styling call, so only a call with an argument is banned for those.
const STYLING_CALLS: [&str; 3] = ["opacity", "cursor", "font"];

/// Banned as `.name` and as `.name_` followed by anything (`rounded_md`, `shadow_lg`).
const STYLING_FAMILIES: [&str; 2] = ["rounded", "shadow"];

pub const THEME_OF_A_CONTEXT: &str = "cx.theme()";
pub const THEME_OF_ANYTHING: &str = ".theme()";
pub const THEME_TRAIT: &str = "ActiveTheme";
pub const THEME_GLOBAL: &str = "Theme::global(";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Colour,
    Role,
    Styling,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub krate: String,
    pub file: String,
    pub line: usize,
    pub rule: Rule,
    pub what: String,
}

impl Finding {
    pub fn said(&self) -> String {
        let Finding {
            krate,
            file,
            line,
            what,
            ..
        } = self;
        let place = format!("{krate}/src/{file}:{line}");
        match self.rule {
            Rule::Colour => format!("{place}: writes `{what}`"),
            Rule::Role => format!("{place}: reads `{what}`"),
            Rule::Styling => format!("{place}: styles with `{what}`"),
        }
    }
}

pub struct Source {
    pub krate: String,
    pub file: String,
    pub text: String,
}

fn is_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn blanked(c: char) -> char {
    if c == '\n' { '\n' } else { ' ' }
}

/// Where a raw string opens at `at`: the number of `#` and the index of its body.
fn raw_string_at(chars: &[char], at: usize) -> Option<(usize, usize)> {
    let starts_a_word = |at: usize| at == 0 || !is_name(chars[at - 1]);
    let r = match chars[at] {
        'r' if starts_a_word(at) => at,
        'b' if chars.get(at + 1) == Some(&'r') && starts_a_word(at) => at + 1,
        _ => return None,
    };
    let hashes = chars[r + 1..].iter().take_while(|&&c| c == '#').count();
    (chars.get(r + 1 + hashes) == Some(&'"')).then_some((hashes, r + hashes + 2))
}

/// The source with its comments and the insides of its string and character literals blanked.
/// The lines stay where they were, so a line number of the result is one of the source.
pub fn code_of(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    while at < chars.len() {
        let next = chars.get(at + 1).copied();
        if chars[at] == '/' && next == Some('/') {
            while at < chars.len() && chars[at] != '\n' {
                out.push(' ');
                at += 1;
            }
        } else if chars[at] == '/' && next == Some('*') {
            let mut depth = 0;
            while at < chars.len() {
                let pair = (chars[at], chars.get(at + 1).copied());
                if pair == ('/', Some('*')) {
                    depth += 1;
                    out.push_str("  ");
                    at += 2;
                } else if pair == ('*', Some('/')) {
                    depth -= 1;
                    out.push_str("  ");
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blanked(chars[at]));
                    at += 1;
                }
            }
        } else if let Some((hashes, body)) = raw_string_at(&chars, at) {
            out.extend(chars[at..body].iter().map(|_| ' '));
            at = body;
            let closing: Vec<char> = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            while at < chars.len() && !chars[at..].starts_with(&closing) {
                out.push(blanked(chars[at]));
                at += 1;
            }
            let end = (at + closing.len()).min(chars.len());
            out.extend(chars[at..end].iter().map(|_| ' '));
            at = end;
        } else if chars[at] == '"' {
            out.push('"');
            at += 1;
            while at < chars.len() && chars[at] != '"' {
                let width = if chars[at] == '\\' { 2 } else { 1 };
                for &c in chars[at..(at + width).min(chars.len())].iter() {
                    out.push(blanked(c));
                }
                at += width;
            }
            if at < chars.len() {
                out.push('"');
                at += 1;
            }
        } else if chars[at] == '\'' {
            // A lifetime has no closing quote right after one character.
            let close = match next {
                // Past the escaped character, which may itself be the quote: `'\''`.
                Some('\\') => chars
                    .get(at + 3..)
                    .and_then(|rest| rest.iter().position(|&c| c == '\''))
                    .map(|p| at + 3 + p),
                Some(c) if c != '\'' && chars.get(at + 2) == Some(&'\'') => Some(at + 2),
                _ => None,
            };
            match close {
                Some(close) => {
                    out.push('\'');
                    out.extend(chars[at + 1..close].iter().map(|_| ' '));
                    out.push('\'');
                    at = close + 1;
                }
                None => {
                    out.push('\'');
                    at += 1;
                }
            }
        } else {
            out.push(chars[at]);
            at += 1;
        }
    }
    out
}

fn code_lines(code: &str) -> impl Iterator<Item = (usize, &str)> {
    code.lines().enumerate().map(|(at, line)| (at + 1, line))
}

fn found<'a>(line: &'a str, pattern: &'a str) -> impl Iterator<Item = usize> + 'a {
    line.match_indices(pattern)
        .map(|(at, _)| at)
        .filter(|&at| !line[..at].chars().next_back().is_some_and(is_name))
}

pub fn function_span(source: &str, opening: &str) -> Option<RangeInclusive<usize>> {
    let code = code_of(source);
    let first = code.lines().position(|line| line.contains(opening))? + 1;
    let last = code
        .lines()
        .enumerate()
        .skip(first)
        .find(|(_, line)| *line == "}")
        .map_or(usize::MAX, |(at, _)| at + 1);
    Some(first..=last)
}

pub fn colours_written(source: &str) -> Vec<(usize, &'static str)> {
    let code = code_of(source);
    let mut hits = Vec::new();
    for (number, line) in code_lines(&code) {
        for pattern in COLOUR_WRITES {
            if found(line, pattern).next().is_some() {
                hits.push((number, pattern));
            }
        }
    }
    hits
}

pub fn colours_not_allowed(folder: &str, file: &str, source: &str) -> Vec<(usize, &'static str)> {
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
pub fn roles_unknown(
    folder: &str,
    file: &str,
    source: &str,
    roles: &BTreeSet<&str>,
) -> Vec<(usize, String)> {
    let may_write = (folder, file) == WRITES_THE_KITS_COLOURS;
    let code = code_of(source);
    let mut hits = Vec::new();
    for (number, line) in code_lines(&code) {
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

pub fn roles() -> BTreeSet<&'static str> {
    KIT_ROLES.into_iter().collect()
}

fn family_of(name: &str) -> Option<&'static str> {
    STYLING_FAMILIES.into_iter().find(|family| {
        name.strip_prefix(family)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('_'))
    })
}

/// The styling a pane may not do itself: what the line calls, once per banned name.
pub fn styling_banned(source: &str) -> Vec<(usize, &'static str)> {
    let code = code_of(source);
    let mut hits = Vec::new();
    for (number, line) in code_lines(&code) {
        let mut on_line: Vec<&'static str> = Vec::new();
        for (start, _) in line.char_indices() {
            if line[..start].chars().next_back().is_some_and(is_name) {
                continue;
            }
            let name = name_at(&line[start..]);
            if name.is_empty() {
                continue;
            }
            let before = &line[..start];
            let member = before.ends_with('.') || before.ends_with("::");
            let hit = if name == THEME_TRAIT {
                Some(THEME_TRAIT)
            } else if name == "Theme" && line[start + name.len()..].starts_with("::global(") {
                Some(THEME_GLOBAL)
            } else if name == "FontWeight"
                && (line[start + name.len()..].starts_with("::")
                    || line[start + name.len()..].trim_start().starts_with('('))
            {
                Some("FontWeight")
            } else if name == "font"
                && !before.ends_with('.')
                && !before.trim_end().ends_with("fn")
                && line[start + name.len()..].trim_start().starts_with('(')
            {
                Some("font")
            } else if !member {
                None
            } else if before.ends_with('.')
                && let Some(call) = STYLING_CALLS.into_iter().find(|call| *call == name)
                && let Some(args) = line[start + name.len()..].trim_start().strip_prefix('(')
                && (call == "opacity" || !args.trim_start().starts_with(')'))
            {
                Some(call)
            } else if let Some(banned) = STYLING_NAMES.into_iter().find(|banned| *banned == name) {
                Some(banned)
            } else if let Some(family) = family_of(name) {
                Some(family)
            } else if name == "theme"
                && before.ends_with('.')
                && line[start + name.len()..].trim_start().starts_with('(')
            {
                let receiver = &before[..before.len() - 1];
                let of_a_context = receiver.ends_with("cx")
                    && !receiver[..receiver.len() - 2]
                        .chars()
                        .next_back()
                        .is_some_and(is_name);
                Some(if of_a_context {
                    THEME_OF_A_CONTEXT
                } else {
                    THEME_OF_ANYTHING
                })
            } else {
                None
            };
            if let Some(hit) = hit
                && !on_line.contains(&hit)
            {
                on_line.push(hit);
                hits.push((number, hit));
            }
        }
    }
    hits
}

/// Every check, over the given sources: colour, role and styling rules hold in every drawing crate.
pub fn scan(sources: &[Source]) -> Vec<Finding> {
    let roles = roles();
    let mut findings = Vec::new();
    for Source { krate, file, text } in sources {
        let finding = |line, rule, what: String| Finding {
            krate: krate.clone(),
            file: file.clone(),
            line,
            rule,
            what,
        };
        for (line, what) in colours_not_allowed(krate, file, text) {
            findings.push(finding(line, Rule::Colour, what.to_string()));
        }
        for (line, what) in roles_unknown(krate, file, text, &roles) {
            findings.push(finding(line, Rule::Role, what));
        }
        for (line, what) in styling_banned(text) {
            findings.push(finding(line, Rule::Styling, what.to_string()));
        }
    }
    findings
}

fn rust_files(folder: &Path, below: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(folder.join(below)) else {
        return;
    };
    for entry in entries {
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

/// Every `.rs` below `src/` of each crate that draws, found by walking `root`, a folder that
/// holds the crates by their folder names. A crate with no `src/` there yields nothing.
pub fn sources_under(root: &Path) -> Vec<Source> {
    let mut sources = Vec::new();
    for krate in CRATES {
        let src = root.join(krate).join("src");
        let mut files = Vec::new();
        rust_files(&src, "", &mut files);
        files.sort();
        for file in files {
            let text = fs::read_to_string(src.join(&file)).expect("a source file can be read");
            sources.push(Source {
                krate: krate.to_string(),
                file,
                text,
            });
        }
    }
    sources
}

/// The crates of the repository, as they stand.
pub fn crates_folder() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}
