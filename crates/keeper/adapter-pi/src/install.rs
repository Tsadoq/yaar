//! The wrapper and the extension the Adapter writes into its own folder of the app.

use std::io;
use std::path::Path;

use yaar_agent::ShellEntry;

/// Writes the wrapper and the extension under `folder` and names the wrapper's folder for
/// the shells of the app: see [`yaar_agent::Adapter::install`] of [`crate::Pi`].
///
/// A hook of the contract of slice 7 that writes nothing and gives no entry, so that a
/// Keeper starts as it did with one Adapter: the package of pi's wrapper fills it.
pub(crate) fn write(folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
    let _ = (folder, companions);
    Ok(Vec::new())
}
