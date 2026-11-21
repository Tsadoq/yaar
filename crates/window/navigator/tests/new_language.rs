//! A new language added by one row and no change of code answers hover (LSP-06, NFR-002).
//!
//! The first test adds a row to the built-in table as text and asks a hover of a file of that
//! language. The second reads the Navigator's own source: no string literal of it names a
//! language, a server, an extension or a file a built-in row names, so that a language is
//! never a branch of the code that starts servers or answers.

mod support;

use std::collections::BTreeSet;
use std::path::Path;

use support::*;
use yaar_navigator::{Answered, BUILTIN, Registry, State, Tree};

#[test]
fn a_language_added_by_one_row_answers_hover() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository("project", &[("main.sil", "def alpha\nuse alpha\n")]);
    // The built-in table, and one row more as text: the stand-in's language, found on the
    // machine.
    let table = format!("{BUILTIN}\n{}", standin_row("standin", &[], "answer", ""));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&table));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    let hover = running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 2, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: alpha".to_owned()),
            guessed: false
        }
    );
    running.end();
}

/// Each source file of the Navigator that is not a test program: written out, and the test
/// below asserts that the list is every such file, so that a new module is not missed.
const SOURCES: [(&str, &str); 14] = [
    ("answers.rs", include_str!("../src/answers.rs")),
    ("clock.rs", include_str!("../src/clock.rs")),
    ("download.rs", include_str!("../src/download.rs")),
    ("driver.rs", include_str!("../src/driver.rs")),
    ("fetch.rs", include_str!("../src/fetch.rs")),
    ("lib.rs", include_str!("../src/lib.rs")),
    ("lifecycle.rs", include_str!("../src/lifecycle.rs")),
    ("lsp.rs", include_str!("../src/lsp.rs")),
    ("navigator.rs", include_str!("../src/navigator.rs")),
    ("old_side.rs", include_str!("../src/old_side.rs")),
    ("port.rs", include_str!("../src/port.rs")),
    ("readiness.rs", include_str!("../src/readiness.rs")),
    ("registry.rs", include_str!("../src/registry.rs")),
    ("search.rs", include_str!("../src/search.rs")),
];

/// Names that must stand in the table and in no string of the source, matched as whole
/// tokens: a token is a run of letters, digits, `_` and `-`.
const TOKENS: [&str; 29] = [
    "typescript",
    "typescriptreact",
    "javascript",
    "javascriptreact",
    "python",
    "rust",
    "tsgo",
    "tsc",
    "typescript-language-server",
    "vtsls",
    "basedpyright",
    "basedpyright-langserver",
    "pyright",
    "pyright-langserver",
    "ty",
    "pyrefly",
    "rust-analyzer",
    "ts",
    "tsx",
    "mts",
    "cts",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "py",
    "pyi",
    "rs",
    "node_modules",
];

/// Names matched as text anywhere in a string.
const TEXTS: [&str; 11] = [
    "TypeScript and JavaScript",
    "tsconfig.json",
    "jsconfig.json",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "Cargo.toml",
    ".venv",
    "@typescript",
];

/// The text of every string literal of `source`, the part of it before its unit tests,
/// comments left out. What is cut off is the unit-test module and nothing else: a
/// `#[cfg(test)]` item before product code would hide that code from the scan.
fn string_literals(source: &str) -> Vec<String> {
    let (code, tests) = source
        .split_once("\n#[cfg(test)]")
        .unwrap_or((source, "\nmod tests"));
    assert!(
        tests.trim_start().starts_with("mod tests"),
        "the first #[cfg(test)] of a source file opens its unit-test module, not another item"
    );
    let chars: Vec<char> = code.chars().collect();
    let mut literals = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let here = chars[at];
        let next = chars.get(at + 1).copied();
        match (here, next) {
            ('/', Some('/')) => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            ('/', Some('*')) => {
                at += 2;
                while at + 1 < chars.len() && !(chars[at] == '*' && chars[at + 1] == '/') {
                    at += 1;
                }
                at += 2;
            }
            // A character literal, which may be a quote; a lifetime is no literal.
            ('\'', Some('\\')) => {
                at += 2;
                while at < chars.len() && chars[at] != '\'' {
                    at += 1;
                }
                at += 1;
            }
            ('\'', Some(_)) if chars.get(at + 2) == Some(&'\'') => at += 3,
            ('r' | 'b', _) if is_raw_start(&chars, at) => {
                let mut start = at + 1;
                if chars[start] == 'r' {
                    start += 1;
                }
                let hashes = chars[start..].iter().take_while(|c| **c == '#').count();
                let open = start + hashes + 1;
                let close: Vec<char> = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                let end = (open..chars.len())
                    .find(|end| chars[*end..].starts_with(&close))
                    .expect("a raw string ends");
                literals.push(chars[open..end].iter().collect());
                at = end + close.len();
            }
            ('"', _) => {
                let mut text = String::new();
                at += 1;
                while chars[at] != '"' {
                    if chars[at] == '\\' {
                        at += 1;
                    }
                    text.push(chars[at]);
                    at += 1;
                }
                literals.push(text);
                at += 1;
            }
            _ => at += 1,
        }
    }
    literals
}

/// Whether a raw string starts at `at`: `r` or `br`, any number of `#`, then a quote, and
/// no letter before.
fn is_raw_start(chars: &[char], at: usize) -> bool {
    if at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_') {
        return false;
    }
    let mut at = at;
    if chars[at] == 'b' {
        at += 1;
    }
    if chars.get(at) != Some(&'r') {
        return false;
    }
    let rest = &chars[at + 1..];
    let hashes = rest.iter().take_while(|c| **c == '#').count();
    rest.get(hashes) == Some(&'"')
}

/// The tokens of `text`, lowercase.
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_scanner_reads_literals_as_rust_writes_them() {
    let source = "let a = \"one \\\"two\\\"\"; // \"not\"\n\
                  let b = r#\"raw \"quoted\" text\"#; let c = '\"'; let d: &'static str = \"last\";\n\
                  /* \"nor this\" */ let e = b\"bytes\";\n\
                  #[cfg(test)]\nmod tests { fn f() { let _ = \"in the tests\"; } }\n";
    assert_eq!(
        string_literals(source),
        ["one \"two\"", "raw \"quoted\" text", "last", "bytes"]
    );
}

#[test]
fn no_string_of_the_navigators_source_names_a_language_a_server_or_a_file_of_a_row() {
    // The list is every source file but the programs of `src/bin`.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut on_disk: Vec<String> = std::fs::read_dir(&folder)
        .expect("read src")
        .map(|entry| entry.expect("an entry"))
        .filter(|entry| entry.file_type().expect("a file type").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".rs"))
        .collect();
    on_disk.sort();
    let listed: Vec<&str> = SOURCES.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        on_disk, listed,
        "a source file of the Navigator is not in the list of this test, or is gone"
    );

    let forbidden: BTreeSet<&str> = TOKENS.iter().copied().collect();
    let mut hits = Vec::new();
    for (name, source) in SOURCES {
        for literal in string_literals(source) {
            let lower = literal.to_lowercase();
            if tokens(&literal)
                .iter()
                .any(|token| forbidden.contains(token.as_str()))
                || TEXTS
                    .iter()
                    .any(|text| lower.contains(&text.to_lowercase()))
            {
                hits.push(format!("{name}: {literal:?}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a language, a server or a file of a built-in row is named in the source:\n{}",
        hits.join("\n")
    );

    // The names are whole: every name the built-in table gives is among them, so that a
    // row added to it fails here until its names are listed too.
    let named = |name: &str| {
        let lower = name.to_lowercase();
        forbidden.contains(lower.as_str()) || TEXTS.iter().any(|text| text.to_lowercase() == lower)
    };
    let (registry, errors) = Registry::read(BUILTIN, None);
    assert!(errors.is_empty(), "{errors:?}");
    let last = |path: &str| {
        Path::new(path)
            .file_name()
            .map(|part| part.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned())
    };
    let mut unlisted = Vec::new();
    for row in registry.rows() {
        let mut names = vec![row.name.clone(), row.title.clone()];
        names.extend(
            row.extensions
                .iter()
                .flat_map(|(e, id)| [e.clone(), id.clone()]),
        );
        names.extend(row.markers.iter().cloned());
        names.extend(row.old_side.link.iter().cloned());
        for server in &row.servers {
            names.push(server.name.clone());
            names.extend(server.machine.iter().cloned());
            names.extend(server.project.iter().map(|path| last(path)));
            names.extend(
                server
                    .downloads
                    .iter()
                    .map(|download| last(&download.program)),
            );
        }
        unlisted.extend(names.into_iter().filter(|name| !named(name)));
    }
    assert!(
        unlisted.is_empty(),
        "the built-in table names these, which the lists of this test lack: {unlisted:?}"
    );
}
