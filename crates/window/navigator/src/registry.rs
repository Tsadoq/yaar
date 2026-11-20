//! The registry of language servers: one TOML format for the built-in table, [`BUILTIN`],
//! compiled into the app, and for the user's own file, [`USER_FILE`] in the app's data
//! folder (FR-069, FR-071, NFR-002).
//!
//! # The format
//!
//! Each top-level table is one row, named by its key: one language, or a family of
//! languages its servers serve, as `typescript` serves TypeScript and JavaScript. A row is
//! the one entry that adds a language (NFR-002): it holds everything that differs between
//! languages and servers, and the code that starts a server holds no name of either.
//!
//! The keys of a row:
//!
//! | Key | Type | What it is | When left out |
//! | --- | --- | --- | --- |
//! | `title` | string | the name the Window shows, `Rust` | the row's name |
//! | `extensions` | table of strings | file extension, with no dot, to the LSP `languageId` | required in a row of the built-in table, and in a user's row whose name no built-in row has |
//! | `markers` | array of strings | files at a worktree's root that start the row's server when the workspace comes on screen | none: the server starts at the first ask about one of its files |
//! | `old_side.link` | array of strings | names of folders linked, wherever the worktree holds one, into an export at the same place: the dependencies the old code reads (ADR-016 point 7) | none |
//! | `old_side.environment` | table of strings | variables added, for a server of an old side only, after the server's `environment` | none |
//! | `server` | array of tables | the row's servers, in the order the app prefers them within each kind of FR-060 (ADR-015 point 5) | required: a row with none is an error |
//!
//! The keys of a server (`[[<row>.server]]`):
//!
//! | Key | Type | What it is | When left out |
//! | --- | --- | --- | --- |
//! | `name` | string | the server's name, as the offer and the line of its state name it, and the folder its download is unpacked into | required in the built-in table; in the user's file, the row's name |
//! | `command` | array of strings | the user's own server: its program, then its arguments. Only in the user's file, where every server must name it | no registered server |
//! | `project` | array of strings | the project's own copy: paths relative to the worktree's root | none |
//! | `machine` | array of strings | names of a copy installed on the machine: each file of that name in every folder of the search path, in the path's order | none |
//! | `trial` | array of strings | the arguments of the trial run of a candidate; one that ends with success is accepted | no trial: a candidate counts as started when it answers `initialize` |
//! | `args` | array of strings | the arguments a copy found by `project`, `machine` or a download is started with | none |
//! | `ready` | table | the readiness rule, below | `{ kind = "answer" }` |
//! | `capabilities` | table | merged into the client capabilities the driver sends in `initialize`, key by key, the server's winning | nothing added |
//! | `initialization_options` | table | sent as `initializationOptions` | none sent |
//! | `settings` | table | the answer to the server's `workspace/configuration`: for an item with a `section`, the value at that section, its parts split at `.`; for one with none, the whole table; null where there is none | every item null |
//! | `environment` | table of strings | variables added to the environment the server runs in, the server's winning | none |
//! | `download` | array of tables | what the app downloads, one table for each [`Platform`], below. Never in the user's file (FR-073) | nothing to offer |
//!
//! A readiness rule (`ready`) is one of three kinds (ADR-015 point 2):
//! - `{ kind = "answer" }`: ready once the server answered `initialize`;
//! - `{ kind = "progress" }`: ready once one `$/progress` began and every one begun has
//!   ended;
//! - `{ kind = "notification", method = "…", match = { … } }`: ready once a notification
//!   of `method` arrives whose `params` hold every key of `match` with an equal value.
//!
//! A download (`[[<row>.server.download]]`) holds: `platform`, one of [`Platform`]'s
//! names; `version`, a string that is not empty and holds no `/`; `url`, the publisher's
//! own address; `sha256`, the digest of the bytes fetched, exactly 64 lowercase
//! hexadecimal digits; `archive`, `tar.gz` or `gz` ([`Archive`]); `strip`, the number of
//! leading parts of each path dropped from a `tar.gz` (0 when left out); `program`, the
//! server's program inside the unpacked folder, or for `gz` the name the one file is
//! written as; `size`, the number of bytes fetched, above 0. Every key but `strip` is
//! required: a download that lacks one, or whose `sha256` is not 64 lowercase hexadecimal
//! digits, is an error of its row, so that no row can download what no digest guards.
//! Two downloads of one server for the same platform are an error of the row too.
//!
//! The strings of a server's `command`, `project`, `args`, `capabilities`,
//! `initialization_options`, `settings` and `environment`, and of the row's `old_side`, may
//! name these variables, written `{name}`, which are put in when the server starts
//! ([`expand`]): `{root}`, the folder the server serves; `{worktree}`, the folder of the
//! worktree, the same as `{root}` but for an old side; `{target}`, a build folder of the
//! app's own for this server; `{platform}`, the machine's [`Platform`] name; `{servers}`,
//! the folder the server's download is unpacked into. A string that names any other is an
//! error of its row.
//!
//! # The user's file
//!
//! A row of the user's file whose name a built-in row has takes that row's place whole,
//! with `title`, `extensions`, `markers` and `old_side` taken from the built-in row when the
//! user's row names none: the user's servers are the only ones tried for those files, and
//! none is ever downloaded (FR-073). A user's row with a new name adds a language. Every
//! server of a user's row must name `command`, and none may name `download`; a row with a
//! server that breaks either rule is reported and passed over. A user's server that names
//! no `name` is named as its row, so that the least a user writes for a language the
//! built-in table knows is its command:
//!
//! ```toml
//! [[rust.server]]
//! command = ["/path/to/rust-analyzer"]
//! ```
//! A user's server may name
//! `project` and `machine` too, tried after every registered command in the order of
//! FR-060. A user's row is [`Row::registered`].
//!
//! # Reading
//!
//! [`Registry::read`] reads the built-in text and then the user's, row by row (NAV-15): a
//! text that is not TOML at all gives no row and one [`RowError`] with no row name; a row
//! that is not a table, holds a key the format does not have (in the row, a server or a
//! download), a value of the wrong type, a variable the format does not have, or breaks a
//! rule above, is reported in a [`RowError`] and passed over, and every other row is read.

use std::path::Path;

use serde_json::Value;
use toml::de::{DeTable, DeValue};

/// The built-in table: `crates/window/navigator/registry.toml`, compiled in.
pub const BUILTIN: &str = include_str!("../registry.toml");

/// The name of the user's registry in the app's data folder.
pub const USER_FILE: &str = "language-servers.toml";

/// A machine the built-in rows can download for, named as Node and npm name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// macOS on Apple silicon: `darwin-arm64`.
    DarwinArm64,
    /// macOS on Intel: `darwin-x64`.
    DarwinX64,
    /// Linux with glibc on x86-64: `linux-x64`.
    LinuxX64,
    /// Linux with glibc on arm64: `linux-arm64`.
    LinuxArm64,
}

impl Platform {
    /// Every platform, in the order above.
    pub const ALL: [Platform; 4] = [
        Platform::DarwinArm64,
        Platform::DarwinX64,
        Platform::LinuxX64,
        Platform::LinuxArm64,
    ];

    /// The machine this program was built for; none on any other.
    pub fn current() -> Option<Platform> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Some(Platform::DarwinArm64),
            ("macos", "x86_64") => Some(Platform::DarwinX64),
            ("linux", "x86_64") => Some(Platform::LinuxX64),
            ("linux", "aarch64") => Some(Platform::LinuxArm64),
            _ => None,
        }
    }

    /// Its name in a row: `darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`.
    pub fn name(self) -> &'static str {
        match self {
            Platform::DarwinArm64 => "darwin-arm64",
            Platform::DarwinX64 => "darwin-x64",
            Platform::LinuxX64 => "linux-x64",
            Platform::LinuxArm64 => "linux-arm64",
        }
    }

    /// The platform of a name; none for a name that is no platform's.
    pub fn named(name: &str) -> Option<Platform> {
        Platform::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// The kind of file a download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    /// A gzip-compressed tar file: its entries are unpacked, `strip` leading parts of each
    /// path dropped.
    TarGz,
    /// One gzip-compressed file: unpacked to the name `program`, and made executable.
    Gz,
}

/// What a server downloads for one platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// The machine.
    pub platform: Platform,
    /// The version pinned.
    pub version: String,
    /// The publisher's own address of the file.
    pub url: String,
    /// The SHA-256 digest of the bytes fetched, in lowercase hexadecimal.
    pub sha256: String,
    /// The kind of file.
    pub archive: Archive,
    /// Leading parts of each path dropped from a `tar.gz`.
    pub strip: u32,
    /// The server's program in the unpacked folder, relative to it.
    pub program: String,
    /// The number of bytes fetched.
    pub size: u64,
}

/// A readiness rule (ADR-015 point 2).
#[derive(Debug, Clone, PartialEq)]
pub enum Ready {
    /// Ready once the server answered `initialize`.
    Answer,
    /// Ready once one `$/progress` began and every one begun has ended.
    Progress,
    /// Ready once a notification of `method` arrives whose `params` hold every key of
    /// `matches` with an equal value.
    Notification {
        /// The notification's method.
        method: String,
        /// What its parameters must hold.
        matches: serde_json::Map<String, serde_json::Value>,
    },
}

/// What a row says of the old side of a diff (ADR-016 point 7).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OldSide {
    /// Names of folders linked, wherever the worktree holds one, into an export at the
    /// same place.
    pub link: Vec<String>,
    /// Variables added for a server of an old side only.
    pub environment: Vec<(String, String)>,
}

/// One server of a row, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    /// Its name.
    pub name: String,
    /// The user's own server: its program and its arguments.
    pub command: Option<Vec<String>>,
    /// The project's own copy: paths relative to the worktree's root.
    pub project: Vec<String>,
    /// Names of a copy installed on the machine.
    pub machine: Vec<String>,
    /// The arguments of the trial run; none when the server names no `trial`.
    pub trial: Option<Vec<String>>,
    /// The arguments a found copy is started with.
    pub args: Vec<String>,
    /// The readiness rule.
    pub ready: Ready,
    /// Merged into the client capabilities: an object, empty when the server names none.
    pub capabilities: serde_json::Value,
    /// Sent as `initializationOptions`.
    pub initialization_options: Option<serde_json::Value>,
    /// The answer to `workspace/configuration`: an object, empty when the server names
    /// none.
    pub settings: serde_json::Value,
    /// Variables added to the environment the server runs in.
    pub environment: Vec<(String, String)>,
    /// What the app downloads, one for each platform the server names.
    pub downloads: Vec<Download>,
}

impl Server {
    /// The download for `platform`, if the server has one.
    pub fn download(&self, platform: Platform) -> Option<&Download> {
        self.downloads.iter().find(|d| d.platform == platform)
    }
}

/// One row of the registry, read: a language and its servers.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Its name: the table's key.
    pub name: String,
    /// The name the Window shows.
    pub title: String,
    /// Whether it came from the user's file.
    pub registered: bool,
    /// File extension, with no dot, to the LSP `languageId`, in the order written.
    pub extensions: Vec<(String, String)>,
    /// Files at a worktree's root that start the row's server when the workspace comes on
    /// screen.
    pub markers: Vec<String>,
    /// What the old side needs.
    pub old_side: OldSide,
    /// Its servers, in the order written.
    pub servers: Vec<Server>,
}

impl Row {
    /// The first server of the row with a download for `platform`, and that download: what
    /// the offer to download names. None for a registered row.
    pub fn download(&self, platform: Platform) -> Option<(&Server, &Download)> {
        if self.registered {
            return None;
        }
        self.servers
            .iter()
            .find_map(|server| server.download(platform).map(|d| (server, d)))
    }

    /// The LSP `languageId` of a file of this row, by its extension.
    pub fn language_id(&self, path: &Path) -> Option<&str> {
        let extension = path.extension()?.to_str()?;
        self.extensions
            .iter()
            .find(|(e, _)| e == extension)
            .map(|(_, id)| id.as_str())
    }
}

/// Which text a [`RowError`] was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The built-in table.
    Builtin,
    /// The user's file.
    User,
}

/// A row that could not be read, and was passed over; or a text that is not TOML, which
/// gave no row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    /// Where.
    pub source: Source,
    /// The row's name; none when the whole text could not be read.
    pub row: Option<String>,
    /// Why, as a sentence.
    pub reason: String,
}

/// The rows of the built-in table and of the user's file, merged: a user's row in the
/// place of the built-in row of its name, a user's row of a new name after every built-in
/// row, each text's rows in the order written.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Registry {
    rows: Vec<Row>,
}

impl Registry {
    /// Reads `builtin` and then `user`, row by row, as the module doc says. Gives the
    /// registry of every row read, and one [`RowError`] for each row passed over and for a
    /// text that is not TOML, in the order found.
    pub fn read(builtin: &str, user: Option<&str>) -> (Registry, Vec<RowError>) {
        let mut errors = Vec::new();
        let mut rows = Vec::new();
        for (name, node) in nodes(builtin, Source::Builtin, &mut errors) {
            if let Some(row) = read_row(&name, node, Source::Builtin, None, &mut errors) {
                rows.push(row);
            }
        }
        if let Some(user) = user {
            for (name, node) in nodes(user, Source::User, &mut errors) {
                let base = rows.iter().find(|row| row.name == name).cloned();
                let Some(row) = read_row(&name, node, Source::User, base.as_ref(), &mut errors)
                else {
                    continue;
                };
                match rows.iter_mut().find(|r| r.name == name) {
                    Some(place) => *place = row,
                    None => rows.push(row),
                }
            }
        }
        (Registry { rows }, errors)
    }

    /// Every row, in the registry's order.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The row named `name`.
    pub fn row(&self, name: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.name == name)
    }

    /// The row whose `extensions` hold the extension of `path`: the first in the
    /// registry's order. None for a file of a language no row knows.
    pub fn for_path(&self, path: &Path) -> Option<&Row> {
        self.rows.iter().find(|row| row.language_id(path).is_some())
    }
}

/// The values of the variables a row may name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Vars {
    /// `{root}`: the folder the server serves.
    pub root: String,
    /// `{worktree}`: the folder of the worktree.
    pub worktree: String,
    /// `{target}`: a build folder of the app's own for this server.
    pub target: String,
    /// `{platform}`: the machine's platform name; empty where it has none.
    pub platform: String,
    /// `{servers}`: the folder the server's download is unpacked into; empty where nothing
    /// is downloaded.
    pub servers: String,
}

/// The names of the variables a row may name.
pub const VARIABLES: [&str; 5] = ["root", "worktree", "target", "platform", "servers"];

/// `text` with each `{name}` of [`VARIABLES`] replaced by its value in `vars`. `{{` and
/// `}}` stand for `{` and `}`. Gives the name of the first variable that is none of
/// [`VARIABLES`], or of a `{` with no `}`, as the error.
pub fn expand(text: &str, vars: &Vars) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if rest[at..].starts_with("{{") || rest[at..].starts_with("}}") {
            out.push_str(&rest[at..at + 1]);
            rest = &rest[at + 2..];
        } else if rest[at..].starts_with('{') {
            let Some(end) = after.find('}') else {
                return Err(rest[at..].to_string());
            };
            let name = &after[..end];
            out.push_str(match name {
                "root" => &vars.root,
                "worktree" => &vars.worktree,
                "target" => &vars.target,
                "platform" => &vars.platform,
                "servers" => &vars.servers,
                _ => return Err(name.to_string()),
            });
            rest = &after[end + 1..];
        } else {
            // A closing brace on its own is no part of a variable.
            out.push('}');
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// [`expand`] applied to every string inside `value`, keys left as they are.
pub fn expand_json(value: &serde_json::Value, vars: &Vars) -> Result<serde_json::Value, String> {
    Ok(match value {
        Value::String(text) => Value::String(expand(text, vars)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| expand_json(item, vars))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), expand_json(item, vars)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

/// A TOML value with its tables in the order they were written: `toml::Table` is sorted by
/// key, and the order of the rows, of a row's extensions and of an environment is the
/// text's.
enum Node {
    Text(String),
    Whole(i64),
    Real(f64),
    Flag(bool),
    Date(String),
    List(Vec<Node>),
    Map(Vec<(String, Node)>),
}

/// The value of `value` as a [`Node`], its tables put back in the order of their keys in
/// the text (by the key's position there).
fn node_of(value: &DeValue<'_>) -> Result<Node, String> {
    Ok(match value {
        DeValue::String(text) => Node::Text(text.to_string()),
        DeValue::Integer(n) => Node::Whole(
            i64::from_str_radix(n.as_str(), n.radix())
                .map_err(|_| format!("the number {n} is out of range"))?,
        ),
        DeValue::Float(n) => Node::Real(
            n.as_str()
                .parse()
                .map_err(|_| format!("the number {n} cannot be read"))?,
        ),
        DeValue::Boolean(flag) => Node::Flag(*flag),
        DeValue::Datetime(date) => Node::Date(date.to_string()),
        DeValue::Array(items) => Node::List(
            items
                .iter()
                .map(|item| node_of(item.get_ref()))
                .collect::<Result<_, _>>()?,
        ),
        DeValue::Table(table) => Node::Map(entries(table)?),
    })
}

/// Where in the text `key` is first written, as its own key or as a part of the header or
/// key of anything under it: the span the parser records for a key that a later header
/// names again is the later one's.
fn first_written(start: usize, value: &DeValue<'_>) -> usize {
    match value {
        DeValue::Table(table) => table
            .iter()
            .map(|(key, value)| first_written(key.span().start, value.get_ref()))
            .fold(start, usize::min),
        DeValue::Array(items) => items
            .iter()
            .map(|item| first_written(start, item.get_ref()))
            .fold(start, usize::min),
        _ => start,
    }
}

/// The entries of `table`, in the order of their keys in the text.
fn entries(table: &DeTable<'_>) -> Result<Vec<(String, Node)>, String> {
    let mut found = table
        .iter()
        .map(|(key, value)| {
            Ok((
                first_written(key.span().start, value.get_ref()),
                key.get_ref().to_string(),
                node_of(value.get_ref())?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    found.sort_by_key(|(at, _, _)| *at);
    Ok(found
        .into_iter()
        .map(|(_, key, node)| (key, node))
        .collect())
}

/// The top-level entries of `text`, in the order written, each with its value or the
/// reason it could not be read; a text that is not TOML gives none and one error.
fn nodes(
    text: &str,
    source: Source,
    errors: &mut Vec<RowError>,
) -> Vec<(String, Result<Node, String>)> {
    let table = match DeTable::parse(text) {
        Ok(table) => table.into_inner(),
        Err(error) => {
            let line = error
                .span()
                .map_or(0, |span| text[..span.start].matches('\n').count() + 1);
            errors.push(RowError {
                source,
                row: None,
                reason: format!("is not TOML: {} (line {line})", error.message()),
            });
            return Vec::new();
        }
    };
    let mut rows: Vec<_> = table
        .iter()
        .map(|(key, value)| {
            (
                first_written(key.span().start, value.get_ref()),
                key.get_ref().to_string(),
                value,
            )
        })
        .collect();
    rows.sort_by_key(|(at, _, _)| *at);
    rows.into_iter()
        .map(|(_, name, value)| (name, node_of(value.get_ref())))
        .collect()
}

/// What a table's entries are looked up by, with the keys the format has checked.
struct Fields<'a>(&'a [(String, Node)]);

impl<'a> Fields<'a> {
    /// The entries of `node`, which must be a table holding no key outside `allowed`.
    fn of(node: &'a Node, what: &str, allowed: &[&str]) -> Result<Fields<'a>, String> {
        let Node::Map(entries) = node else {
            return Err(format!("{what} is not a table"));
        };
        for (key, _) in entries {
            if !allowed.contains(&key.as_str()) {
                return Err(format!(
                    "{what} holds the key `{key}`, which the format does not have"
                ));
            }
        }
        Ok(Fields(entries))
    }

    fn get(&self, key: &str) -> Option<&'a Node> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, node)| node)
    }
}

fn text_of(node: &Node, what: &str) -> Result<String, String> {
    match node {
        Node::Text(text) => Ok(text.clone()),
        _ => Err(format!("{what} must be a string")),
    }
}

fn texts_of(node: &Node, what: &str) -> Result<Vec<String>, String> {
    match node {
        Node::List(items) => items.iter().map(|item| text_of(item, what)).collect(),
        _ => Err(format!("{what} must be an array of strings")),
    }
}

fn pairs_of(node: &Node, what: &str) -> Result<Vec<(String, String)>, String> {
    match node {
        Node::Map(entries) => entries
            .iter()
            .map(|(key, value)| Ok((key.clone(), text_of(value, what)?)))
            .collect(),
        _ => Err(format!("{what} must be a table of strings")),
    }
}

fn json_of(node: &Node, what: &str) -> Result<Value, String> {
    Ok(match node {
        Node::Text(text) | Node::Date(text) => Value::String(text.clone()),
        Node::Whole(n) => Value::from(*n),
        Node::Real(n) => serde_json::Number::from_f64(*n)
            .map(Value::Number)
            .ok_or_else(|| format!("{what} holds a number JSON cannot hold"))?,
        Node::Flag(flag) => Value::Bool(*flag),
        Node::List(items) => Value::Array(
            items
                .iter()
                .map(|item| json_of(item, what))
                .collect::<Result<_, _>>()?,
        ),
        Node::Map(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| Ok((key.clone(), json_of(item, what)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

/// A table of `node` as a JSON object whose strings name only variables the format has.
fn object_of(node: &Node, what: &str) -> Result<Value, String> {
    if !matches!(node, Node::Map(_)) {
        return Err(format!("{what} must be a table"));
    }
    let value = json_of(node, what)?;
    expand_json(&value, &Vars::default()).map_err(|name| unknown_variable(what, &name))?;
    Ok(value)
}

/// `text` as it is, if it names only variables the format has.
fn checked(text: String, what: &str) -> Result<String, String> {
    expand(&text, &Vars::default()).map_err(|name| unknown_variable(what, &name))?;
    Ok(text)
}

fn checked_texts(node: &Node, what: &str) -> Result<Vec<String>, String> {
    texts_of(node, what)?
        .into_iter()
        .map(|text| checked(text, what))
        .collect()
}

fn checked_pairs(node: &Node, what: &str) -> Result<Vec<(String, String)>, String> {
    pairs_of(node, what)?
        .into_iter()
        .map(|(key, text)| Ok((key, checked(text, what)?)))
        .collect()
}

/// The sentence for an unknown variable, or for a `{` never closed, which [`expand`] tells
/// by the `{` its error starts with: no variable's name can.
fn unknown_variable(what: &str, name: &str) -> String {
    if name.starts_with('{') {
        format!("{what} holds `{name}`, a `{{` with no `}}`")
    } else {
        format!("{what} names `{{{name}}}`, a variable the format does not have")
    }
}

/// The row `name` of the text of `source`, read; `base` is the built-in row of that name
/// when `source` is the user's file. A row that cannot be read is put in `errors` and gives
/// none.
fn read_row(
    name: &str,
    node: Result<Node, String>,
    source: Source,
    base: Option<&Row>,
    errors: &mut Vec<RowError>,
) -> Option<Row> {
    match node.and_then(|node| row_of(name, &node, source, base)) {
        Ok(row) => Some(row),
        Err(reason) => {
            errors.push(RowError {
                source,
                row: Some(name.to_string()),
                reason,
            });
            None
        }
    }
}

fn row_of(name: &str, node: &Node, source: Source, base: Option<&Row>) -> Result<Row, String> {
    let fields = Fields::of(
        node,
        "the row",
        &["title", "extensions", "markers", "old_side", "server"],
    )?;
    let title = match fields.get("title") {
        Some(title) => text_of(title, "`title`")?,
        None => base.map_or_else(|| name.to_string(), |base| base.title.clone()),
    };
    let extensions = match fields.get("extensions") {
        Some(node) => {
            let pairs = pairs_of(node, "`extensions`")?;
            if pairs.is_empty() {
                return Err("names no `extensions`, so it serves no file".to_string());
            }
            pairs
        }
        None => match base {
            Some(base) => base.extensions.clone(),
            None => return Err("names no `extensions`, so it serves no file".to_string()),
        },
    };
    let markers = match fields.get("markers") {
        Some(node) => texts_of(node, "`markers`")?,
        None => base.map(|base| base.markers.clone()).unwrap_or_default(),
    };
    let old_side = match fields.get("old_side") {
        Some(node) => old_side_of(node)?,
        None => base.map(|base| base.old_side.clone()).unwrap_or_default(),
    };
    let servers = match fields.get("server") {
        Some(Node::List(items)) if !items.is_empty() => items
            .iter()
            .enumerate()
            .map(|(at, item)| server_of(name, at, item, source))
            .collect::<Result<Vec<_>, _>>()?,
        Some(Node::List(_)) | None => return Err("names no `server`".to_string()),
        Some(_) => return Err("`server` must be an array of tables".to_string()),
    };
    if source == Source::Builtin {
        // A download is kept in a folder of its server's name.
        for (at, server) in servers.iter().enumerate() {
            if servers[..at].iter().any(|other| other.name == server.name) {
                return Err(format!("names two servers `{}`", server.name));
            }
        }
    }
    Ok(Row {
        name: name.to_string(),
        title,
        registered: source == Source::User,
        extensions,
        markers,
        old_side,
        servers,
    })
}

fn old_side_of(node: &Node) -> Result<OldSide, String> {
    let fields = Fields::of(node, "`old_side`", &["link", "environment"])?;
    Ok(OldSide {
        link: match fields.get("link") {
            Some(node) => checked_texts(node, "`old_side.link`")?,
            None => Vec::new(),
        },
        environment: match fields.get("environment") {
            Some(node) => checked_pairs(node, "`old_side.environment`")?,
            None => Vec::new(),
        },
    })
}

fn server_of(row: &str, at: usize, node: &Node, source: Source) -> Result<Server, String> {
    let fields = Fields::of(
        node,
        &format!("server {}", at + 1),
        &[
            "name",
            "command",
            "project",
            "machine",
            "trial",
            "args",
            "ready",
            "capabilities",
            "initialization_options",
            "settings",
            "environment",
            "download",
        ],
    )?;
    let name = match (fields.get("name"), source) {
        (Some(name), _) => text_of(name, "`name`")?,
        (None, Source::User) => row.to_string(),
        (None, Source::Builtin) => return Err(format!("server {} has no `name`", at + 1)),
    };
    let what = format!("server `{name}`");
    let list = |key: &str| match fields.get(key) {
        Some(node) => checked_texts(node, &format!("{what}'s `{key}`")),
        None => Ok(Vec::new()),
    };

    let command = match (fields.get("command"), source) {
        (Some(node), Source::User) => {
            let command = checked_texts(node, &format!("{what}'s `command`"))?;
            if command.first().is_none_or(|program| program.is_empty()) {
                return Err(format!("{what}'s `command` names no program"));
            }
            Some(command)
        }
        (Some(_), Source::Builtin) => {
            return Err(format!(
                "{what} names a `command`, which only the user's file may"
            ));
        }
        (None, Source::User) => {
            return Err(format!(
                "{what} has no `command`, which every server of the user's file needs"
            ));
        }
        (None, Source::Builtin) => None,
    };
    if source == Source::User && fields.get("download").is_some() {
        return Err(format!(
            "{what} names a `download`, and no server the user registered is downloaded"
        ));
    }

    let machine = match fields.get("machine") {
        Some(node) => texts_of(node, &format!("{what}'s `machine`"))?,
        None => Vec::new(),
    };
    let trial = match fields.get("trial") {
        Some(node) => Some(texts_of(node, &format!("{what}'s `trial`"))?),
        None => None,
    };
    let ready = match fields.get("ready") {
        Some(node) => ready_of(node, &what)?,
        None => Ready::Answer,
    };
    let capabilities = match fields.get("capabilities") {
        Some(node) => object_of(node, &format!("{what}'s `capabilities`"))?,
        None => Value::Object(Default::default()),
    };
    let initialization_options = match fields.get("initialization_options") {
        Some(node) => Some(object_of(
            node,
            &format!("{what}'s `initialization_options`"),
        )?),
        None => None,
    };
    let settings = match fields.get("settings") {
        Some(node) => object_of(node, &format!("{what}'s `settings`"))?,
        None => Value::Object(Default::default()),
    };
    let environment = match fields.get("environment") {
        Some(node) => checked_pairs(node, &format!("{what}'s `environment`"))?,
        None => Vec::new(),
    };
    let mut downloads: Vec<Download> = Vec::new();
    match fields.get("download") {
        Some(Node::List(items)) => {
            for (n, item) in items.iter().enumerate() {
                let download = download_of(&what, n, item)?;
                if downloads.iter().any(|d| d.platform == download.platform) {
                    return Err(format!(
                        "{what} has two downloads for {}",
                        download.platform.name()
                    ));
                }
                downloads.push(download);
            }
        }
        Some(_) => return Err(format!("{what}'s `download` must be an array of tables")),
        None => {}
    }

    Ok(Server {
        name,
        command,
        project: list("project")?,
        machine,
        trial,
        args: list("args")?,
        ready,
        capabilities,
        initialization_options,
        settings,
        environment,
        downloads,
    })
}

fn ready_of(node: &Node, what: &str) -> Result<Ready, String> {
    let what = format!("{what}'s `ready`");
    let kind = match Fields::of(node, &what, &["kind", "method", "match"])?.get("kind") {
        Some(kind) => text_of(kind, &format!("{what}'s `kind`"))?,
        None => return Err(format!("{what} names no `kind`")),
    };
    match kind.as_str() {
        "answer" | "progress" => {
            Fields::of(node, &what, &["kind"])?;
            Ok(if kind == "answer" {
                Ready::Answer
            } else {
                Ready::Progress
            })
        }
        "notification" => {
            let fields = Fields::of(node, &what, &["kind", "method", "match"])?;
            let method = match fields.get("method") {
                Some(method) => text_of(method, &format!("{what}'s `method`"))?,
                None => return Err(format!("{what} of kind `notification` names no `method`")),
            };
            let matches = match fields.get("match") {
                Some(node) => match json_of(node, &what)? {
                    Value::Object(map) => map,
                    _ => return Err(format!("{what}'s `match` must be a table")),
                },
                None => Default::default(),
            };
            Ok(Ready::Notification { method, matches })
        }
        other => Err(format!(
            "{what} has the kind `{other}`, and the kinds are `answer`, `progress` and `notification`"
        )),
    }
}

fn download_of(server: &str, at: usize, node: &Node) -> Result<Download, String> {
    let what = format!("download {} of {server}", at + 1);
    let fields = Fields::of(
        node,
        &what,
        &[
            "platform", "version", "url", "sha256", "archive", "strip", "program", "size",
        ],
    )?;
    let text = |key: &str| match fields.get(key) {
        Some(node) => match text_of(node, &format!("{what}'s `{key}`"))? {
            text if text.is_empty() => Err(format!("{what}'s `{key}` is empty")),
            text => Ok(text),
        },
        None => Err(format!("{what} has no `{key}`")),
    };
    let platform = text("platform")?;
    let platform = Platform::named(&platform)
        .ok_or_else(|| format!("{what} names the platform `{platform}`, which is none of ours"))?;
    let version = text("version")?;
    if version.contains('/') {
        return Err(format!("{what}'s `version` holds a `/`"));
    }
    let url = text("url")?;
    let sha256 = text("sha256")?;
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!(
            "{what}'s `sha256` is not 64 lowercase hexadecimal digits, and no download is made that no digest guards"
        ));
    }
    let archive = match text("archive")?.as_str() {
        "tar.gz" => Archive::TarGz,
        "gz" => Archive::Gz,
        other => {
            return Err(format!(
                "{what}'s `archive` is `{other}`, and the kinds are `tar.gz` and `gz`"
            ));
        }
    };
    let strip = match fields.get("strip") {
        None => 0,
        Some(Node::Whole(n)) => u32::try_from(*n)
            .map_err(|_| format!("{what}'s `strip` must be a number of parts, 0 or more"))?,
        Some(_) => return Err(format!("{what}'s `strip` must be a number")),
    };
    let program = text("program")?;
    let size = match fields.get("size") {
        Some(Node::Whole(n)) if *n > 0 => *n as u64,
        Some(Node::Whole(_)) => return Err(format!("{what}'s `size` must be above 0")),
        Some(_) => return Err(format!("{what}'s `size` must be a number")),
        None => return Err(format!("{what} has no `size`")),
    };
    Ok(Download {
        platform,
        version,
        url,
        sha256,
        archive,
        strip,
        program,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The row names of a registry, in its order.
    fn names(registry: &Registry) -> Vec<&str> {
        registry
            .rows()
            .iter()
            .map(|row| row.name.as_str())
            .collect()
    }

    /// The row names of the errors, in the order found; an empty string for none.
    fn error_rows(errors: &[RowError]) -> Vec<&str> {
        errors
            .iter()
            .map(|e| e.row.as_deref().unwrap_or(""))
            .collect()
    }

    /// The reason of the error naming `row`.
    fn reason<'a>(errors: &'a [RowError], row: &str) -> &'a str {
        errors
            .iter()
            .find(|e| e.row.as_deref() == Some(row))
            .map(|e| e.reason.as_str())
            .unwrap_or_else(|| panic!("no error names the row {row}: {errors:?}"))
    }

    fn read_ok(builtin: &str, user: Option<&str>) -> Registry {
        let (registry, errors) = Registry::read(builtin, user);
        assert_eq!(errors, vec![], "the text reads with no error");
        registry
    }

    /// One built-in row that reads, named `name`, with one extension named like it.
    fn good(name: &str) -> String {
        format!(
            "[{name}]\nextensions = {{ {name} = \"{name}\" }}\n\
             [[{name}.server]]\nname = \"{name}-ls\"\nmachine = [\"{name}-ls\"]\n"
        )
    }

    // NAV-15, the built-in table

    #[test]
    fn the_built_in_table_reads_with_no_error_in_the_order_written() {
        let registry = read_ok(BUILTIN, None);

        // Written typescript, python, rust: not the order of the keys.
        assert_eq!(names(&registry), ["typescript", "python", "rust"]);
        let servers = |row: &str| -> Vec<&str> {
            registry
                .row(row)
                .unwrap()
                .servers
                .iter()
                .map(|s| s.name.as_str())
                .collect()
        };
        assert_eq!(
            servers("typescript"),
            ["tsgo", "typescript-language-server", "vtsls"]
        );
        assert_eq!(
            servers("python"),
            ["basedpyright", "pyright", "ty", "pyrefly"]
        );
        assert_eq!(servers("rust"), ["rust-analyzer"]);
        assert!(registry.rows().iter().all(|row| !row.registered));
    }

    #[test]
    fn the_typescript_row_turns_off_the_acquisition_of_types_under_each_section() {
        let registry = read_ok(BUILTIN, None);
        let tsgo = &registry.row("typescript").unwrap().servers[0];
        assert_eq!(tsgo.name, "tsgo");

        for section in ["js/ts", "typescript", "javascript"] {
            // The key under a section is found by its parts, as the server's
            // `workspace/configuration` answer finds it.
            let value = ["tsserver", "automaticTypeAcquisition", "enabled"]
                .iter()
                .try_fold(&tsgo.settings[section], |at, part| at.get(part));
            assert_eq!(value, Some(&json!(false)), "under the section {section}");
        }
    }

    #[test]
    fn the_rust_row_keeps_cargo_offline() {
        let registry = read_ok(BUILTIN, None);
        let server = &registry.row("rust").unwrap().servers[0];
        assert_eq!(server.name, "rust-analyzer");

        let has = |key: &str, value: &str| {
            server
                .environment
                .contains(&(key.to_string(), value.to_string()))
        };
        assert!(has("CARGO_NET_OFFLINE", "true"), "{:?}", server.environment);
        assert!(has("CARGO_TARGET_DIR", "{target}"));
    }

    #[test]
    fn a_built_in_row_reads_into_every_field_the_format_names() {
        let registry = read_ok(BUILTIN, None);
        let typescript = registry.row("typescript").unwrap();
        assert_eq!(typescript.title, "TypeScript and JavaScript");
        assert_eq!(
            typescript.markers,
            ["tsconfig.json", "jsconfig.json", "package.json"]
        );
        assert_eq!(typescript.old_side.link, ["node_modules"]);
        assert_eq!(
            typescript.language_id(Path::new("a/b.tsx")),
            Some("typescriptreact")
        );
        let tsgo = &typescript.servers[0];
        assert_eq!(
            tsgo.project,
            ["node_modules/@typescript/typescript-{platform}/lib/tsc"]
        );
        assert_eq!(tsgo.machine, ["tsgo"]);
        assert_eq!(tsgo.trial.as_deref(), Some(&["--version".to_string()][..]));
        assert_eq!(tsgo.args, ["--lsp", "--stdio"]);
        assert_eq!(tsgo.ready, Ready::Answer);
        assert_eq!(tsgo.command, None);
        assert_eq!(tsgo.downloads.len(), 4);

        // No `trial`, and a section's rule of its own.
        let python = registry.row("python").unwrap();
        assert_eq!(python.servers[0].trial, None);
        let rust = registry.row("rust").unwrap();
        assert_eq!(
            rust.servers[0].ready,
            Ready::Notification {
                method: "experimental/serverStatus".into(),
                matches: json!({ "quiescent": true }).as_object().unwrap().clone(),
            }
        );
        assert_eq!(
            rust.servers[0].capabilities,
            json!({ "experimental": { "serverStatusNotification": true } })
        );
        assert_eq!(
            rust.servers[0].initialization_options,
            Some(json!({ "checkOnSave": false, "cargo": { "targetDir": "{target}" } }))
        );
        // A server with no `capabilities` has an empty object, not none.
        assert_eq!(typescript.servers[0].capabilities, json!({}));
        assert_eq!(typescript.servers[0].initialization_options, None);
        assert_eq!(python.servers[0].settings, json!({}));
        assert_eq!(
            python.servers[1].settings,
            json!({ "python": { "pythonPath": "{root}/.venv/bin/python" } })
        );

        let download = rust.servers[0].download(Platform::DarwinArm64).unwrap();
        assert_eq!(download.archive, Archive::Gz);
        assert_eq!(download.strip, 0, "strip left out is 0");
        assert_eq!(download.program, "rust-analyzer");
        let ty = python.servers[2].download(Platform::LinuxX64).unwrap();
        assert_eq!((ty.archive, ty.strip), (Archive::TarGz, 1));
    }

    // NAV-15, row by row

    #[test]
    fn a_row_that_cannot_be_read_is_reported_and_passed_over_and_the_rows_after_it_are_read() {
        // Rows are named so that the order of the text differs from the order of the keys.
        let text = format!(
            "{zeta}\n\
             [yak]\ncolour = \"red\"\nextensions = {{ y = \"y\" }}\n\
             [[yak.server]]\nname = \"y\"\n\
             {mid}\n\
             [xylem]\nextensions = {{ x = \"x\" }}\nmarkers = \"x\"\n\
             [[xylem.server]]\nname = \"x\"\n\
             {alpha}\n\
             {beta}\n\
             [vole]\nextensions = {{ v = \"v\" }}\n\
             [[vole.server]]\nname = \"v\"\nargs = [\"--at\", \"{{nothere}}\"]\n\
             {gamma}\n\
             [uakari]\n\
             [[uakari.server]]\nname = \"u\"\n\
             {delta}\n",
            zeta = good("zeta"),
            mid = good("mid"),
            alpha = good("alpha"),
            beta = good("beta"),
            gamma = good("gamma"),
            delta = good("delta"),
        );
        // A key after a table would belong to that table: `wolf` goes first, to be a row.
        let text = format!("wolf = 1\n{text}");
        let (registry, errors) = Registry::read(&text, None);

        assert_eq!(
            names(&registry),
            ["zeta", "mid", "alpha", "beta", "gamma", "delta"]
        );
        assert_eq!(
            error_rows(&errors),
            ["wolf", "yak", "xylem", "vole", "uakari"]
        );
        assert!(errors.iter().all(|e| e.source == Source::Builtin));
        assert!(reason(&errors, "wolf").contains("not a table"));
        assert!(reason(&errors, "yak").contains("colour"));
        assert!(reason(&errors, "xylem").contains("markers"));
        assert!(reason(&errors, "vole").contains("nothere"));
        assert!(reason(&errors, "uakari").contains("extensions"));
    }

    #[test]
    fn a_row_that_names_a_variable_it_knows_is_read_and_a_doubled_brace_is_a_brace() {
        let text = "[v]\nextensions = { v = \"v\" }\n\
            [[v.server]]\nname = \"v\"\n\
            args = [\"{root}\", \"{worktree}\", \"{target}\", \"{platform}\", \"{servers}\", \"{{x}}\"]\n\
            project = [\"{root}/a\"]\n\
            environment = { A = \"{target}/b\" }\n\
            settings = { s = { t = [\"{root}\"] } }\n\
            [v.old_side]\nlink = [\"{worktree}/m\"]\nenvironment = { B = \"{worktree}\" }\n";
        let registry = read_ok(text, None);
        assert_eq!(names(&registry), ["v"]);
    }

    #[test]
    fn a_variable_the_format_does_not_have_is_an_error_of_its_row_wherever_it_is_written() {
        // Each text puts one unknown variable in one place the format expands.
        let places = [
            ("args", "args = [\"{nothere}\"]"),
            ("project", "project = [\"{nothere}\"]"),
            ("environment", "environment = { A = \"{nothere}\" }"),
            ("settings", "settings = { a = { b = \"{nothere}\" } }"),
            ("capabilities", "capabilities = { a = [\"{nothere}\"] }"),
            (
                "initialization_options",
                "initialization_options = { a = \"{nothere}\" }",
            ),
            ("an unclosed brace", "args = [\"{root\"]"),
        ];
        for (place, line) in places {
            let text = format!(
                "[v]\nextensions = {{ v = \"v\" }}\n[[v.server]]\nname = \"v\"\n{line}\n{}",
                good("ok")
            );
            let (registry, errors) = Registry::read(&text, None);
            assert_eq!(names(&registry), ["ok"], "{place}");
            assert_eq!(error_rows(&errors), ["v"], "{place}");
        }
        let old_side = "[v]\nextensions = { v = \"v\" }\n[[v.server]]\nname = \"v\"\n\
                        [v.old_side]\nlink = [\"{nothere}\"]\n";
        assert_eq!(Registry::read(old_side, None).1.len(), 1);
        let old_environment = "[v]\nextensions = { v = \"v\" }\n[[v.server]]\nname = \"v\"\n\
                        [v.old_side]\nenvironment = { A = \"{nothere}\" }\n";
        assert_eq!(Registry::read(old_environment, None).1.len(), 1);
    }

    #[test]
    fn a_text_that_is_not_toml_gives_no_row_and_one_error_with_no_row_name() {
        let (registry, errors) = Registry::read("[rust\nextensions = ", None);
        assert_eq!(names(&registry), Vec::<&str>::new());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].row, None);
        assert_eq!(errors[0].source, Source::Builtin);
        assert!(!errors[0].reason.is_empty());

        // The user's file broken: the built-in rows stay, and the error says whose it is.
        let builtin = good("a");
        let (registry, errors) = Registry::read(&builtin, Some("[[[ nonsense"));
        assert_eq!(names(&registry), ["a"]);
        assert_eq!(errors.len(), 1);
        assert_eq!(
            (errors[0].source, errors[0].row.as_deref()),
            (Source::User, None)
        );
    }

    #[test]
    fn a_row_with_no_server_and_one_with_nothing_but_a_title_are_errors() {
        let text = "[a]\nextensions = { a = \"a\" }\n\
                    [b]\nextensions = { b = \"b\" }\nserver = []\n\
                    [c]\nextensions = {}\n[[c.server]]\nname = \"c\"\n\
                    [d]\nextensions = { d = \"d\" }\nserver = [1]\n";
        let (registry, errors) = Registry::read(text, None);
        assert_eq!(names(&registry), Vec::<&str>::new());
        assert_eq!(error_rows(&errors), ["a", "b", "c", "d"]);
        assert!(reason(&errors, "a").contains("server"));
        assert!(reason(&errors, "c").contains("extensions"));
    }

    #[test]
    fn a_value_of_the_wrong_type_is_an_error_of_its_row() {
        let wrongs = [
            "title = 3",
            "extensions = { a = 3 }",
            "extensions = [\"a\"]",
            "old_side = \"x\"",
            "old_side = { link = \"x\" }",
            "old_side = { colour = [] }",
            "server = { name = \"a\" }",
        ];
        for wrong in wrongs {
            // A row with `extensions` first, so that only the line under test can be wrong.
            let head = if wrong.starts_with("extensions") {
                ""
            } else {
                "extensions = { a = \"a\" }\n"
            };
            let server = if wrong.starts_with("server") {
                ""
            } else {
                "[[a.server]]\nname = \"a\"\n"
            };
            let text = format!("[a]\n{head}{wrong}\n{server}");
            let (registry, errors) = Registry::read(&text, None);
            assert_eq!(names(&registry), Vec::<&str>::new(), "{wrong}");
            assert_eq!(error_rows(&errors), ["a"], "{wrong}");
        }
    }

    #[test]
    fn the_extensions_markers_and_servers_keep_the_order_written() {
        let text = "[z]\nmarkers = [\"b\", \"a\"]\n\
                    [z.extensions]\nzz = \"1\"\naa = \"2\"\nmm = \"3\"\n\
                    [[z.server]]\nname = \"s2\"\n\
                    [[z.server]]\nname = \"s1\"\n\
                    [z.server.environment]\nZ = \"1\"\nA = \"2\"\n\
                    [[z.server]]\nname = \"s0\"\n\
                    [a]\nextensions = { a = \"a\" }\n[[a.server]]\nname = \"a\"\n";
        let registry = read_ok(text, None);

        assert_eq!(names(&registry), ["z", "a"]);
        let z = registry.row("z").unwrap();
        assert_eq!(z.markers, ["b", "a"]);
        let ext: Vec<_> = z.extensions.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(ext, ["zz", "aa", "mm"]);
        let servers: Vec<_> = z.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(servers, ["s2", "s1", "s0"]);
        let env: Vec<_> = z.servers[1]
            .environment
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(env, ["Z", "A"]);
    }

    #[test]
    fn a_row_is_where_its_name_is_first_written_even_when_its_own_header_comes_later() {
        // The row's first mention is a header of something under it; its own header is
        // written after another row's.
        let by_servers = "[[z.server]]\nname = \"z\"\nmachine = [\"z\"]\n\
                          [b]\nextensions = { b = \"b\" }\n[[b.server]]\nname = \"b\"\nmachine = [\"b\"]\n\
                          [z]\nextensions = { z = \"z\" }\n";
        let by_extensions = "[z.extensions]\nz = \"z\"\n\
                             [b]\nextensions = { b = \"b\" }\n[[b.server]]\nname = \"b\"\nmachine = [\"b\"]\n\
                             [z]\n[[z.server]]\nname = \"z\"\nmachine = [\"z\"]\n";
        for text in [by_servers, by_extensions] {
            let registry = read_ok(text, None);
            assert_eq!(names(&registry), ["z", "b"], "{text}");
        }
    }

    #[test]
    fn the_json_values_of_a_server_keep_their_types() {
        let text = "[a]\nextensions = { a = \"a\" }\n\
                    [[a.server]]\nname = \"a\"\n\
                    [a.server.settings]\nn = 3\nf = 1.5\nneg = -2\nb = true\ns = \"x\"\n\
                    list = [1, \"two\", [3.5], { four = 4 }]\n\
                    [a.server.settings.deep.er]\nk = false\n\
                    [a.server.initialization_options]\nflag = true\n";
        let registry = read_ok(text, None);
        let server = &registry.row("a").unwrap().servers[0];
        assert_eq!(
            server.settings,
            json!({
                "n": 3, "f": 1.5, "neg": -2, "b": true, "s": "x",
                "list": [1, "two", [3.5], { "four": 4 }],
                "deep": { "er": { "k": false } },
            })
        );
        assert_eq!(server.initialization_options, Some(json!({ "flag": true })));
    }

    #[test]
    fn a_readiness_rule_reads_in_each_of_its_three_kinds_and_a_bad_one_is_an_error() {
        let row = |name: &str, ready: &str| {
            format!(
                "[{name}]\nextensions = {{ {name} = \"{name}\" }}\n\
                 [[{name}.server]]\nname = \"{name}\"\n{ready}\n"
            )
        };
        let text = [
            row("a", "ready = { kind = \"answer\" }"),
            row("b", "ready = { kind = \"progress\" }"),
            row(
                "c",
                "ready = { kind = \"notification\", method = \"m/n\", match = { ok = true, n = 2 } }",
            ),
            row("d", "ready = { kind = \"notification\", method = \"m/n\" }"),
            row("e", "ready = { kind = \"bogus\" }"),
            row("f", "ready = { kind = \"notification\" }"),
            row("g", "ready = { kind = \"answer\", method = \"x\" }"),
            row("h", "ready = { }"),
            row("i", "ready = \"answer\""),
            row(
                "j",
                "ready = { kind = \"notification\", method = \"m\", match = 1 }",
            ),
            row("k", ""),
        ]
        .concat();
        let (registry, errors) = Registry::read(&text, None);

        let ready = |name: &str| registry.row(name).unwrap().servers[0].ready.clone();
        assert_eq!(ready("a"), Ready::Answer);
        assert_eq!(ready("b"), Ready::Progress);
        assert_eq!(
            ready("c"),
            Ready::Notification {
                method: "m/n".into(),
                matches: json!({ "ok": true, "n": 2 }).as_object().unwrap().clone(),
            }
        );
        assert_eq!(
            ready("d"),
            Ready::Notification {
                method: "m/n".into(),
                matches: Default::default(),
            }
        );
        assert_eq!(ready("k"), Ready::Answer, "left out is the answer");
        assert_eq!(error_rows(&errors), ["e", "f", "g", "h", "i", "j"]);
    }

    // NAV-15, downloads

    /// A built-in row `a` with one server holding `download` tables written as `downloads`.
    fn with_downloads(downloads: &str) -> String {
        format!("[a]\nextensions = {{ a = \"a\" }}\n[[a.server]]\nname = \"a\"\n{downloads}\n")
    }

    const DIGEST: &str = "902e2fe1cf0799198ef902c6b8c310a450fef629a6baba41d45641ef75c04ebd";

    fn download(lines: &[(&str, String)]) -> String {
        let mut text = String::from("[[a.server.download]]\n");
        for (key, value) in lines {
            text.push_str(&format!("{key} = {value}\n"));
        }
        text
    }

    /// The seven keys of a download that reads, with one of them replaced or taken out.
    fn download_with(key: &str, value: Option<&str>) -> String {
        let mut lines: Vec<(&str, String)> = vec![
            ("platform", "\"linux-x64\"".into()),
            ("version", "\"1.2\"".into()),
            ("url", "\"https://example.org/a.tgz\"".into()),
            ("sha256", format!("\"{DIGEST}\"")),
            ("archive", "\"tar.gz\"".into()),
            ("strip", "1".into()),
            ("program", "\"bin/a\"".into()),
            ("size", "42".into()),
        ];
        lines.retain(|(k, _)| *k != key);
        if let Some(value) = value {
            lines.push((key, value.to_string()));
        }
        download(&lines)
    }

    #[test]
    fn a_download_reads_into_every_key() {
        let registry = read_ok(&with_downloads(&download_with("strip", None)), None);
        let server = &registry.row("a").unwrap().servers[0];
        assert_eq!(
            server.downloads,
            [Download {
                platform: Platform::LinuxX64,
                version: "1.2".into(),
                url: "https://example.org/a.tgz".into(),
                sha256: DIGEST.into(),
                archive: Archive::TarGz,
                strip: 0,
                program: "bin/a".into(),
                size: 42,
            }]
        );
        assert!(server.download(Platform::DarwinArm64).is_none());
    }

    #[test]
    fn a_download_that_lacks_a_key_or_breaks_a_rule_is_an_error_of_its_row() {
        let upper = DIGEST.replacen('e', "E", 1);
        let cases: Vec<(&str, String)> = vec![
            ("no sha256", download_with("sha256", None)),
            (
                "63 digits",
                download_with("sha256", Some(&format!("\"{}\"", &DIGEST[1..]))),
            ),
            (
                "65 digits",
                download_with("sha256", Some(&format!("\"{DIGEST}0\""))),
            ),
            (
                "an upper-case digit",
                download_with("sha256", Some(&format!("\"{upper}\""))),
            ),
            (
                "a digit that is none",
                download_with("sha256", Some(&format!("\"{}g\"", &DIGEST[1..]))),
            ),
            ("no size", download_with("size", None)),
            ("size 0", download_with("size", Some("0"))),
            ("a size below 0", download_with("size", Some("-3"))),
            ("a size that is text", download_with("size", Some("\"42\""))),
            ("no platform", download_with("platform", None)),
            (
                "an unknown platform",
                download_with("platform", Some("\"windows-x64\"")),
            ),
            ("no version", download_with("version", None)),
            ("an empty version", download_with("version", Some("\"\""))),
            (
                "a version with a slash",
                download_with("version", Some("\"1/2\"")),
            ),
            ("no url", download_with("url", None)),
            ("no archive", download_with("archive", None)),
            (
                "an unknown archive",
                download_with("archive", Some("\"zip\"")),
            ),
            ("no program", download_with("program", None)),
            ("a strip below 0", download_with("strip", Some("-1"))),
            ("an unknown key", download_with("mirror", Some("\"x\""))),
        ];
        for (what, downloads) in cases {
            let text = format!("{}{}", with_downloads(&downloads), good("ok"));
            let (registry, errors) = Registry::read(&text, None);
            assert_eq!(names(&registry), ["ok"], "{what}");
            assert_eq!(error_rows(&errors), ["a"], "{what}");
            assert!(!reason(&errors, "a").is_empty(), "{what}");
        }
    }

    #[test]
    fn two_downloads_of_one_server_for_one_platform_are_an_error_of_the_row() {
        let two = format!(
            "{}{}",
            download_with("strip", None),
            download_with("version", Some("\"2.0\""))
        );
        let (registry, errors) = Registry::read(&with_downloads(&two), None);
        assert_eq!(names(&registry), Vec::<&str>::new());
        assert!(reason(&errors, "a").contains("linux-x64"));

        let other = download_with("platform", Some("\"linux-arm64\""));
        let ok = format!("{}{}", download_with("strip", None), other);
        assert_eq!(
            read_ok(&with_downloads(&ok), None)
                .row("a")
                .unwrap()
                .servers[0]
                .downloads
                .len(),
            2
        );
    }

    // NAV-15, a server's own keys

    #[test]
    fn a_built_in_server_needs_a_name_and_may_not_name_a_command() {
        let text = "[a]\nextensions = { a = \"a\" }\n[[a.server]]\nmachine = [\"a\"]\n\
                    [b]\nextensions = { b = \"b\" }\n[[b.server]]\nname = \"b\"\ncommand = [\"/bin/b\"]\n\
                    [c]\nextensions = { c = \"c\" }\n[[c.server]]\nname = \"c\"\ncolour = 1\n\
                    [d]\nextensions = { d = \"d\" }\n[[d.server]]\nname = \"same\"\n[[d.server]]\nname = \"same\"\n";
        let (registry, errors) = Registry::read(text, None);
        assert_eq!(names(&registry), Vec::<&str>::new());
        assert_eq!(error_rows(&errors), ["a", "b", "c", "d"]);
        assert!(reason(&errors, "a").contains("name"));
        assert!(reason(&errors, "b").contains("command"));
        assert!(reason(&errors, "c").contains("colour"));
        assert!(reason(&errors, "d").contains("same"));
    }

    // NAV-15, the user's file

    const BUILTIN_RUST: &str = "[rust]\ntitle = \"Rust\"\nmarkers = [\"Cargo.toml\"]\n\
        [rust.extensions]\nrs = \"rust\"\n\
        [rust.old_side]\nlink = [\"target\"]\n\
        [[rust.server]]\nname = \"rust-analyzer\"\nmachine = [\"rust-analyzer\"]\n\
        [[rust.server.download]]\nplatform = \"linux-x64\"\nversion = \"1\"\nurl = \"https://e.org/r.gz\"\n\
        sha256 = \"902e2fe1cf0799198ef902c6b8c310a450fef629a6baba41d45641ef75c04ebd\"\n\
        archive = \"gz\"\nprogram = \"rust-analyzer\"\nsize = 5\n";

    #[test]
    fn a_users_row_of_a_built_in_name_replaces_it_whole_and_takes_what_it_leaves_out() {
        let builtin = format!("{}{}{}", good("first"), BUILTIN_RUST, good("last"));
        let user = "[rust]\n[[rust.server]]\nname = \"mine\"\ncommand = [\"/opt/ra\", \"--x\"]\n\
                    [[rust.server]]\ncommand = [\"/opt/ra2\"]\nproject = [\"p/ra\"]\nmachine = [\"ra\"]\n";
        let registry = read_ok(&builtin, Some(user));

        // In the built-in row's place, not after the rows that follow it.
        assert_eq!(names(&registry), ["first", "rust", "last"]);
        let rust = registry.row("rust").unwrap();
        assert!(rust.registered);
        assert_eq!(rust.title, "Rust");
        assert_eq!(rust.extensions, [("rs".to_string(), "rust".to_string())]);
        assert_eq!(rust.markers, ["Cargo.toml"]);
        assert_eq!(rust.old_side.link, ["target"]);
        // None of the built-in servers, and none ever downloaded.
        assert_eq!(rust.servers.len(), 2);
        assert_eq!(rust.servers[0].name, "mine");
        assert_eq!(
            rust.servers[0].command,
            Some(vec!["/opt/ra".into(), "--x".into()])
        );
        assert!(rust.servers.iter().all(|s| s.downloads.is_empty()));
        // A server with no name is named as its row; it may name project and machine too.
        assert_eq!(rust.servers[1].name, "rust");
        assert_eq!(rust.servers[1].project, ["p/ra"]);
        assert_eq!(rust.servers[1].machine, ["ra"]);
        // The built-in row of another name is as it was.
        assert!(!registry.row("first").unwrap().registered);
    }

    #[test]
    fn a_users_row_that_names_its_own_title_extensions_markers_and_old_side_keeps_them() {
        let user = "[rust]\ntitle = \"Mine\"\nmarkers = [\"m\"]\n\
                    [rust.extensions]\nrsx = \"rust\"\n[rust.old_side]\nlink = [\"x\"]\n\
                    [[rust.server]]\ncommand = [\"/opt/ra\"]\n";
        let registry = read_ok(BUILTIN_RUST, Some(user));
        let rust = registry.row("rust").unwrap();
        assert_eq!(rust.title, "Mine");
        assert_eq!(rust.markers, ["m"]);
        assert_eq!(rust.extensions, [("rsx".to_string(), "rust".to_string())]);
        assert_eq!(rust.old_side.link, ["x"]);
    }

    #[test]
    fn the_least_a_user_writes_is_a_command_and_it_reads_as_one_registered_server() {
        let user = "[[rust.server]]\ncommand = [\"/path/to/rust-analyzer\"]\n";
        let registry = read_ok(BUILTIN_RUST, Some(user));
        let rust = registry.row("rust").unwrap();

        assert!(rust.registered);
        assert_eq!(rust.servers.len(), 1);
        assert_eq!(rust.servers[0].name, "rust");
        assert_eq!(
            rust.servers[0].command,
            Some(vec!["/path/to/rust-analyzer".to_string()])
        );
        assert_eq!(rust.servers[0].ready, Ready::Answer);
        assert!(
            rust.download(Platform::LinuxX64).is_none(),
            "no offer for it"
        );
    }

    #[test]
    fn a_users_row_that_names_a_download_or_lacks_a_command_is_passed_over_and_the_built_in_row_stays()
     {
        let builtin = format!("{}{}", BUILTIN_RUST, good("go"));
        let user = "[rust]\n[[rust.server]]\ncommand = [\"/x\"]\n\
                    [[rust.server.download]]\nplatform = \"linux-x64\"\nversion = \"1\"\nurl = \"https://e.org/r.gz\"\n\
                    sha256 = \"902e2fe1cf0799198ef902c6b8c310a450fef629a6baba41d45641ef75c04ebd\"\n\
                    archive = \"gz\"\nprogram = \"x\"\nsize = 5\n\
                    [go]\n[[go.server]]\nname = \"g\"\nmachine = [\"g\"]\n\
                    [empty]\nextensions = { e = \"e\" }\n[[empty.server]]\ncommand = []\n\
                    [ok]\nextensions = { o = \"o\" }\n[[ok.server]]\ncommand = [\"/ok\"]\n";
        let (registry, errors) = Registry::read(&builtin, Some(user));

        assert_eq!(error_rows(&errors), ["rust", "go", "empty"]);
        assert!(errors.iter().all(|e| e.source == Source::User));
        assert!(reason(&errors, "rust").contains("download"));
        assert!(reason(&errors, "go").contains("command"));
        assert_eq!(names(&registry), ["rust", "go", "ok"]);
        assert!(
            !registry.row("rust").unwrap().registered,
            "the built-in one stays"
        );
        assert!(!registry.row("go").unwrap().registered);
        assert!(
            registry
                .row("rust")
                .unwrap()
                .download(Platform::LinuxX64)
                .is_some()
        );
    }

    #[test]
    fn a_users_row_of_a_new_name_comes_after_the_built_in_rows_and_needs_its_extensions() {
        let builtin = format!("{}{}", good("zeta"), good("alpha"));
        let user = "[standin]\ntitle = \"Stand-in\"\nmarkers = [\"main.sil\"]\n\
                    [standin.extensions]\nsil = \"standin\"\n\
                    [[standin.server]]\nname = \"mine\"\ncommand = [\"/s\", \"--log\", \"l\"]\n\
                    ready = { kind = \"notification\", method = \"standin/status\", match = { ready = true } }\n\
                    [aardvark]\n[[aardvark.server]]\ncommand = [\"/a\"]\n\
                    [bee]\nextensions = { b = \"b\" }\n[[bee.server]]\ncommand = [\"/b\"]\n";
        let (registry, errors) = Registry::read(&builtin, Some(user));

        assert_eq!(names(&registry), ["zeta", "alpha", "standin", "bee"]);
        assert_eq!(error_rows(&errors), ["aardvark"]);
        let standin = registry.row("standin").unwrap();
        assert!(standin.registered);
        assert_eq!(standin.title, "Stand-in");
        assert_eq!(standin.servers[0].name, "mine");
        assert_eq!(
            standin.language_id(Path::new("x/main.sil")),
            Some("standin")
        );
        assert_eq!(
            registry.for_path(Path::new("main.sil")).unwrap().name,
            "standin"
        );
        // A title left out is the row's name.
        assert_eq!(registry.row("bee").unwrap().title, "bee");
    }

    // Variables

    fn vars() -> Vars {
        Vars {
            root: "/r".into(),
            worktree: "/w".into(),
            target: "/t".into(),
            platform: "darwin-arm64".into(),
            servers: "/s".into(),
        }
    }

    #[test]
    fn every_variable_of_the_format_is_put_in() {
        assert_eq!(
            expand("{root}|{worktree}|{target}|{platform}|{servers}", &vars()),
            Ok("/r|/w|/t|darwin-arm64|/s".to_string())
        );
        assert_eq!(
            expand("{root}/node_modules/{platform}/{root}", &vars()),
            Ok("/r/node_modules/darwin-arm64//r".to_string())
        );
        assert_eq!(
            expand("no braces at all", &vars()),
            Ok("no braces at all".to_string())
        );
        assert_eq!(expand("", &vars()), Ok(String::new()));
        assert_eq!(expand("{root}", &Vars::default()), Ok(String::new()));
        assert_eq!(expand("ünï {root} ✓", &vars()), Ok("ünï /r ✓".to_string()));
    }

    #[test]
    fn a_doubled_brace_stands_for_one_and_names_no_variable() {
        assert_eq!(expand("{{root}}", &vars()), Ok("{root}".to_string()));
        assert_eq!(expand("{{{root}}}", &vars()), Ok("{/r}".to_string()));
        assert_eq!(expand("a}}b{{c", &vars()), Ok("a}b{c".to_string()));
        // A lone closing brace is no part of a variable: it stays as it is.
        assert_eq!(expand("a}b", &vars()), Ok("a}b".to_string()));
    }

    #[test]
    fn a_name_that_is_no_variable_and_a_brace_never_closed_are_errors() {
        assert_eq!(expand("{nothere}", &vars()), Err("nothere".to_string()));
        assert_eq!(expand("{root}/{Root}", &vars()), Err("Root".to_string()));
        assert_eq!(expand("a {root} {b} {c}", &vars()), Err("b".to_string()));
        assert_eq!(expand("{}", &vars()), Err(String::new()));
        assert_eq!(expand("{ root }", &vars()), Err(" root ".to_string()));
        assert_eq!(expand("x{root", &vars()), Err("{root".to_string()));
        assert_eq!(expand("{", &vars()), Err("{".to_string()));
    }

    #[test]
    fn a_value_put_in_is_not_read_again() {
        let vars = Vars {
            root: "{target}".into(),
            ..vars()
        };
        assert_eq!(expand("{root}", &vars), Ok("{target}".to_string()));
    }

    #[test]
    fn every_string_inside_a_json_value_is_expanded_and_its_keys_are_not() {
        let value = json!({
            "{root}": "{root}/a",
            "list": ["{platform}", 3, true, null, 1.5, { "{target}": ["{servers}"] }],
            "n": 7,
        });
        assert_eq!(
            expand_json(&value, &vars()),
            Ok(json!({
                "{root}": "/r/a",
                "list": ["darwin-arm64", 3, true, null, 1.5, { "{target}": ["/s"] }],
                "n": 7,
            }))
        );
        assert_eq!(expand_json(&json!("{root}"), &vars()), Ok(json!("/r")));
        assert_eq!(expand_json(&json!(null), &vars()), Ok(json!(null)));
        assert_eq!(
            expand_json(&json!({ "a": [{ "b": "{nothere}" }] }), &vars()),
            Err("nothere".to_string())
        );
    }
}
