//! Removes the folder of every session `runs` says no longer runs, under
//! `<folder>/`[`REVIEWS`]`/`; with `written_before`, only where every file in it is older
//! (a file written exactly then is not), so a session resumed later keeps its files.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use crate::deliver::{REVIEWS, is_folder_name};

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
        // An error is noted and the walk goes on, but std's listing gives nothing after an error:
        // the sessions not reached stay for the next clean-up.
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failure.get_or_insert(error);
                continue;
            }
        };
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

fn is_before(found: &fs::Metadata, written_before: Option<SystemTime>) -> io::Result<bool> {
    match written_before {
        None => Ok(true),
        Some(time) => Ok(found.modified()? < time),
    }
}
