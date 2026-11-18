//! Switchboard (`H-EVENTS, H-SESSION`).
//!
//! Owns: what hooks and extensions report, held while no window is attached; and for each agent session its CLI, terminal, review and state.
//!
//! Surface: none yet. This crate is a shell: it marks where the component lives so that
//! the layout rules of `cargo xtask layout` guard it from the first commit.
