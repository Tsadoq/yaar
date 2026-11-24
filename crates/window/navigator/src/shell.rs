//! The environment of the language servers, taken once from the user's login shell.
//!
//! A Window started from a terminal has the terminal's environment, and its servers run in
//! it. A Window started from the Finder or the Dock on macOS, or from a desktop's menu on
//! Linux, has the system's short search path: no server of rustup, Homebrew or nvm is found,
//! and a downloaded rust-analyzer finds no `cargo`. The Dock's terminals start the user's
//! login shell and so have what the user's start-up files set; the servers take the same,
//! by running that shell once ([`take`]) and reading the environment it prints.
//!
//! # When
//!
//! Never before a server is needed: the Navigator runs the shell when it first has to
//! search for a row's candidates (`lifecycle.rs`), not at its start, and not for a Window
//! that never shows a workspace with a server's file. Once per Navigator: the answer, or the
//! failure, holds for its life. While the shell runs, the keys that wait for a search are
//! held (`lifecycle.rs`, "Where a key stands"): their state is `Starting` with no process,
//! and asks about their files are answered by text match, as for any server not ready.
//! The shell runs on a thread of its own and reports with `Event::Shell`; the Navigator's
//! thread never waits for it.
//!
//! How the shell is run, what it gives and when it fails are the module [`yaar_link::login`]'s
//! to say: the Keeper takes the same environment by the same rules, and this module
//! re-exports what the Navigator names.

pub use yaar_link::login::{
    COMMAND, LoginShell, MARKER, SHELL_DEADLINE, ShellError, ShellGroup as Group, search_path,
    take, take_in,
};
