//! Dock (`H-TERM`).
//!
//! Owns: pseudo-terminals and their children, the ring of recent output, the environment of each shell, plain quit and stop-everything quit, the watch for the turn marks in the terminal bytes.
//!
//! Surface: none yet. This crate is a shell: it marks where the component lives so that
//! the layout rules of `cargo xtask layout` guard it from the first commit.
