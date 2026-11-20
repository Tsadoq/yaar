//! `cargo xtask language-servers <folder>`: fills `<folder>` with the real language servers
//! the Navigator's built-in table pins for this machine, through the app's own downloader
//! (`yaar_navigator::download::fill`), so that the tests that need a real server find them
//! where `YAAR_LANGUAGE_SERVERS` names (`crates/window/navigator/tests/support/mod.rs`).
//!
//! Run once on a machine, by hand on a Mac or in the Linux machine, and by a step of the
//! workflow on GitHub's, whose cache keeps the folder between runs, keyed on the table. A
//! row whose pinned version and digest the folder holds already is left as it is, and
//! nothing is fetched for it: a second run fetches nothing. It prints one line a row, and
//! ends with failure, saying why, when a download fails or the machine is none the table
//! downloads for. It fetches from the publishers' own addresses, as the app does, and
//! from nothing else.

use std::path::Path;
use std::process::ExitCode;

/// Fills `folder`, as the module doc says.
pub fn fill(folder: &Path) -> ExitCode {
    let _ = (folder, yaar_navigator::BUILTIN);
    todo!()
}
