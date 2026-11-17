//! Store (`H-STORE`).
//!
//! Owns: the SQLite database and its one writer: reviews, sessions, steps of turns, marks, settings.
//!
//! Surface: none yet. This crate is a shell: it marks where the component lives so that
//! the layout rules of `cargo xtask layout` guard it from the first commit.
