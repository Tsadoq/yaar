//! The language servers' environment, taken once from the user's login shell: a Window started from
//! the Finder or a menu has only the system's short search path, so no rustup, Homebrew or nvm
//! server is found. How the shell is run is `yaar_link::login`'s.

pub use yaar_link::login::{
    COMMAND, LoginShell, MARKER, SHELL_DEADLINE, ShellError, ShellGroup as Group, search_path,
    take, take_in,
};
