//! The clean-up of the files of long reviews: the body of the Adapter's
//! `remove_files_of_ended`.
//!
//! The Adapter writes a review too long to ride inline into `<folder>/`[`REVIEWS`]`/<CLI
//! session id>/` (see `deliver.rs`), and nothing else ever removes it. A call removes the
//! folder of every session `runs` says no longer runs, and with `written_before` only where
//! every file in it was last modified before that time (a file written exactly then is not
//! before it), so that a session ended a moment ago and resumed later still finds its files.
//!
//! Nothing outside the folder of reviews is touched and no link is followed: a link inside a
//! session's folder, a session's folder that is itself a link, and a folder of reviews that
//! is itself a link are each dealt with by their own metadata, as `symlink_metadata` gives
//! it, and the link goes, never what it names. What is not a session's folder is left alone:
//! a plain file beside the sessions, a name that is not text, a name no session id of this
//! Adapter can be ([`crate::deliver`] writes ASCII letters, digits, `-` and `_` only).
//!
//! The bytes given are the blocks the removed files and links used on disk (`st_blocks`,
//! in units of 512 bytes), the unit the recorded states are counted in: the folders
//! themselves count for nothing. A session whose folder cannot be removed stays for the next
//! clean-up: the others go on, and the first failure is what the call gives at the end.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use crate::deliver::REVIEWS;

/// Whether `name` can be the id of a session of this Adapter, which the delivery writes as a
/// folder name: ASCII letters, digits, `-` and `_`, as Claude Code's session ids are.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Removes the files of the sessions of `folder` that no longer run, as the module doc says;
/// gives the bytes removed.
pub(crate) fn remove_files_of_ended(
    folder: &Path,
    runs: &dyn Fn(&str) -> bool,
    written_before: Option<SystemTime>,
) -> io::Result<u64> {
    let reviews = folder.join(REVIEWS);
    // A link is no folder of reviews of ours: `read_dir` would follow it.
    match fs::symlink_metadata(&reviews) {
        Ok(found) if found.is_dir() => {}
        Ok(_) => return Ok(0),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    }
    let mut removed = 0;
    let mut failure = None;
    for entry in fs::read_dir(&reviews)? {
        let entry = entry?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_folder_name(&id) || runs(&id) {
            continue;
        }
        match remove_session(&entry.path(), written_before) {
            Ok(bytes) => removed += bytes,
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    match failure {
        None => Ok(removed),
        Some(error) => Err(error),
    }
}

/// Removes the session's folder at `path`, or the link that stands in its place, unless it
/// holds a file written at `written_before` or later; gives the bytes removed. What is no
/// folder and no link (a plain file) is no session and stays.
fn remove_session(path: &Path, written_before: Option<SystemTime>) -> io::Result<u64> {
    let found = fs::symlink_metadata(path)?;
    if found.is_dir() {
        let mut bytes = 0;
        if !all_before(path, written_before, &mut bytes)? {
            return Ok(0);
        }
        fs::remove_dir_all(path)?;
        Ok(bytes)
    } else if found.is_symlink() {
        if !is_before(&found, written_before)? {
            return Ok(0);
        }
        fs::remove_file(path)?;
        Ok(found.blocks() * 512)
    } else {
        Ok(0)
    }
}

/// Whether every file and link under the folder `path` was last modified before
/// `written_before`, adding to `bytes` the blocks they use. Never follows a link.
fn all_before(
    path: &Path,
    written_before: Option<SystemTime>,
    bytes: &mut u64,
) -> io::Result<bool> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let found = fs::symlink_metadata(entry.path())?;
        if found.is_dir() {
            if !all_before(&entry.path(), written_before, bytes)? {
                return Ok(false);
            }
        } else {
            if !is_before(&found, written_before)? {
                return Ok(false);
            }
            *bytes += found.blocks() * 512;
        }
    }
    Ok(true)
}

/// Whether the file `found` describes was last modified before `written_before`; always,
/// when there is no time.
fn is_before(found: &fs::Metadata, written_before: Option<SystemTime>) -> io::Result<bool> {
    match written_before {
        None => Ok(true),
        Some(time) => Ok(found.modified()? < time),
    }
}
