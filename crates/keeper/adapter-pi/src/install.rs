//! The extension the Adapter writes into its own folder of the app, and the variable every
//! shell of the app gets for it.

use std::ffi::OsString;
use std::io;
use std::path::Path;

use yaar_agent::ShellEntry;

use crate::{EXTENSION, EXTENSION_FILE, EXTENSION_SOURCE, REPORT_PROGRAM, REPORT_VARIABLE};

/// Writes the extension under `folder` and names the reporting program for the shells of
/// the app: see [`yaar_agent::Adapter::install`] of [`crate::Pi`]. A path that is not text
/// is an error of kind `InvalidInput`, and nothing is written.
///
/// Nothing of pi's is written here: pi loads the extension only once the user said yes,
/// and only [`crate::Pi`]'s answer to the question writes the entry that makes it so.
pub(crate) fn write(folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
    // Taken first, so that a program path that cannot be named leaves nothing behind.
    let program = std::path::absolute(companions.join(REPORT_PROGRAM))?;
    if program.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the path {} is not text, which the shells of the app are given",
                program.display()
            ),
        ));
    }

    write_extension(&folder.join(EXTENSION))?;

    Ok(vec![ShellEntry::Variable {
        name: OsString::from(REPORT_VARIABLE),
        value: program.into_os_string(),
    }])
}

/// Writes the extension's source as `folder/yaar.ts` through a temporary file of the
/// folder that is renamed into place, so that pi never loads half a file, and a start of
/// the Keeper that overlaps pi's own load of it finds the whole old text or the whole new.
fn write_extension(folder: &Path) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let temporary = folder.join(format!(".{EXTENSION_FILE}.{}.tmp", std::process::id()));
    let written = std::fs::write(&temporary, EXTENSION_SOURCE)
        .and_then(|()| std::fs::rename(&temporary, folder.join(EXTENSION_FILE)));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}
