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
//! | `name` | string | the server's name, as the offer and the line of its state name it, and the folder its download is unpacked into | required |
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
//! names; `version`; `url`, the publisher's own address; `sha256`, the digest of the bytes
//! fetched, 64 lowercase hexadecimal digits; `archive`, `tar.gz` or `gz` ([`Archive`]);
//! `strip`, the number of leading parts of each path dropped from a `tar.gz` (0 when left
//! out); `program`, the server's program inside the unpacked folder, or for `gz` the name
//! the one file is written as; `size`, the number of bytes fetched.
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
//! server that breaks either rule is reported and passed over. A user's server may name
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
        let _ = (builtin, user);
        todo!()
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
    let _ = (text, vars);
    todo!()
}

/// [`expand`] applied to every string inside `value`, keys left as they are.
pub fn expand_json(value: &serde_json::Value, vars: &Vars) -> Result<serde_json::Value, String> {
    let _ = (value, vars);
    todo!()
}
