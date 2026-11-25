use std::ops::Range;
use std::path::Path;

use gpui_kit::Hsla;

use crate::tokens::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    Keyword,
    Function,
    String,
    Number,
    Comment,
}

pub fn colour(kind: SyntaxKind, palette: &Palette) -> Hsla {
    match kind {
        SyntaxKind::Keyword => palette.syn_keyword,
        SyntaxKind::Function => palette.syn_function,
        SyntaxKind::String => palette.syn_string,
        SyntaxKind::Number => palette.syn_number,
        SyntaxKind::Comment => palette.syn_comment,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Script,
    Python,
    Go,
    C,
    Shell,
    Data,
}

fn language_of(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Language::Script),
        "py" | "pyi" => Some(Language::Python),
        "go" => Some(Language::Go),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "swift" | "java" | "kt" => {
            Some(Language::C)
        }
        "sh" | "bash" | "zsh" => Some(Language::Shell),
        "json" | "toml" | "yaml" | "yml" => Some(Language::Data),
        _ => None,
    }
}

fn is_keyword(lang: Language, word: &str) -> bool {
    match lang {
        Language::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        Language::Script => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "from"
                | "function"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "instanceof"
                | "interface"
                | "let"
                | "new"
                | "null"
                | "of"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "return"
                | "static"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "var"
                | "void"
                | "while"
                | "yield"
        ),
        Language::Python => matches!(
            word,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        Language::Go => matches!(
            word,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "false"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "nil"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "true"
                | "type"
                | "var"
        ),
        Language::C => matches!(
            word,
            "auto"
                | "bool"
                | "break"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "float"
                | "for"
                | "func"
                | "if"
                | "import"
                | "inline"
                | "int"
                | "let"
                | "long"
                | "namespace"
                | "new"
                | "nil"
                | "null"
                | "nullptr"
                | "private"
                | "public"
                | "return"
                | "short"
                | "signed"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "template"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typedef"
                | "union"
                | "unsigned"
                | "using"
                | "var"
                | "virtual"
                | "void"
                | "volatile"
                | "while"
        ),
        Language::Shell => matches!(
            word,
            "case"
                | "do"
                | "done"
                | "elif"
                | "else"
                | "esac"
                | "export"
                | "fi"
                | "for"
                | "function"
                | "if"
                | "in"
                | "local"
                | "return"
                | "set"
                | "then"
                | "until"
                | "while"
        ),
        Language::Data => matches!(word, "true" | "false" | "null"),
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn uses_hash_comment(lang: Language) -> bool {
    matches!(lang, Language::Python | Language::Shell | Language::Data)
}

fn uses_slash_comment(lang: Language) -> bool {
    matches!(
        lang,
        Language::Rust | Language::Script | Language::Go | Language::C
    )
}

/// Tokenises one line of code into sorted, non-overlapping byte ranges for syntax colouring.
pub fn highlight_line(path: &Path, line: &str) -> Vec<(Range<usize>, SyntaxKind)> {
    let Some(lang) = language_of(path) else {
        return Vec::new();
    };
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut expect_fn = false;

    let trimmed = line.trim_start();
    if uses_slash_comment(lang) && trimmed.starts_with('*') {
        let start = len - trimmed.len();
        out.push((start..len, SyntaxKind::Comment));
        return out;
    }

    while i < len {
        let b = bytes[i];

        if uses_slash_comment(lang) && b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            out.push((i..len, SyntaxKind::Comment));
            break;
        }
        if uses_slash_comment(lang) && b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = if i + 1 < len { i + 2 } else { len };
            out.push((start..i, SyntaxKind::Comment));
            continue;
        }
        if uses_hash_comment(lang) && b == b'#' {
            out.push((i..len, SyntaxKind::Comment));
            break;
        }

        if b == b'"' || b == b'`' || (b == b'\'' && lang != Language::Rust) {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            out.push((start..i, SyntaxKind::String));
            continue;
        }

        if b == b'\'' && lang == Language::Rust {
            // Character literal (`'a'`, `'\n'`), not a lifetime (`'a`).
            if i + 2 < len && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                out.push((i..i + 3, SyntaxKind::String));
                i += 3;
                continue;
            }
            if i + 3 < len && bytes[i + 1] == b'\\' {
                let mut end = i + 2;
                while end < len && bytes[end] != b'\'' {
                    end += 1;
                }
                if end < len {
                    out.push((i..end + 1, SyntaxKind::String));
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
            continue;
        }

        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            out.push((start..i, SyntaxKind::Number));
            continue;
        }

        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &line[start..i];
            if is_keyword(lang, word) {
                expect_fn = matches!(word, "fn" | "function" | "def" | "func");
                out.push((start..i, SyntaxKind::Keyword));
            } else {
                let is_call =
                    i < len && (bytes[i] == b'(' || (lang == Language::Rust && bytes[i] == b'!'));
                let is_type = matches!(
                    lang,
                    Language::Rust | Language::Script | Language::Go | Language::C
                ) && b.is_ascii_uppercase()
                    && word.bytes().any(|c| c.is_ascii_lowercase());
                if expect_fn || is_call || is_type {
                    out.push((start..i, SyntaxKind::Function));
                }
                expect_fn = false;
            }
            continue;
        }

        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }

    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{SyntaxKind, highlight_line};

    fn kinds<'a>(path: &str, line: &'a str) -> Vec<(&'a str, SyntaxKind)> {
        highlight_line(Path::new(path), line)
            .into_iter()
            .map(|(range, kind)| (&line[range], kind))
            .collect()
    }

    #[test]
    fn highlights_typescript_keywords_types_functions_strings_numbers_and_comments() {
        let got = kinds(
            "src/cart.ts",
            r#"export function total(to: Currency = "EUR"): number { // 42"#,
        );
        assert_eq!(
            got,
            [
                ("export", SyntaxKind::Keyword),
                ("function", SyntaxKind::Keyword),
                ("total", SyntaxKind::Function),
                ("Currency", SyntaxKind::Function),
                ("\"EUR\"", SyntaxKind::String),
                ("// 42", SyntaxKind::Comment),
            ]
        );
    }

    #[test]
    fn highlights_rust_lines_without_treating_lifetimes_as_strings() {
        let got = kinds(
            "src/lib.rs",
            "pub fn parse<'a>(n: usize) -> &'a str { 0xff }",
        );
        assert_eq!(
            got,
            [
                ("pub", SyntaxKind::Keyword),
                ("fn", SyntaxKind::Keyword),
                ("parse", SyntaxKind::Function),
                ("0xff", SyntaxKind::Number),
            ]
        );
    }

    #[test]
    fn unknown_extensions_produce_no_spans() {
        assert!(highlight_line(Path::new("notes.txt"), "fn main() { 1 }").is_empty());
    }

    #[test]
    fn handles_multibyte_utf8_safely() {
        let line = "const s = \"€12.50\"; // café";
        let spans = highlight_line(Path::new("money.ts"), line);
        for (range, _) in &spans {
            assert!(line.is_char_boundary(range.start));
            assert!(line.is_char_boundary(range.end));
        }
        assert_eq!(spans.len(), 3);
    }
}
