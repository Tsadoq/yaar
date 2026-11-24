use std::ffi::OsString;
use std::io;
use std::path::Path;

use yaar_agent::ShellEntry;

use crate::{EXTENSION, EXTENSION_FILE, EXTENSION_SOURCE, REPORT_PROGRAM, REPORT_VARIABLE};

/// A path that is not text is `InvalidInput` and nothing is written. Writes no pi setting:
/// pi loads the extension only after the user agreed.
pub(crate) fn write(folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
    // Taken first: an unnameable program path leaves nothing behind.
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

/// Renamed into place so pi never loads half a file and an overlapping Keeper start finds
/// the whole old text or the whole new.
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
