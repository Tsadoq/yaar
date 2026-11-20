//! The extension the Adapter writes into its own folder of the app, and the variable every
//! shell of the app gets for it.

use std::io;
use std::path::Path;

use yaar_agent::ShellEntry;

/// Writes the extension under `folder` and names the reporting program for the shells of
/// the app: see [`yaar_agent::Adapter::install`] of [`crate::Pi`]. A path that is not text
/// is an error of kind `InvalidInput`, and nothing is written.
///
/// A hook of the contract of slice 7 that writes nothing and gives no entry, so that a
/// Keeper starts as it did with one Adapter: the package of pi's reporting program and
/// installing fills it.
pub(crate) fn write(folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
    let _ = (folder, companions);
    Ok(Vec::new())
}
