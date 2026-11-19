//! A copy of git's list of the files of a worktree, for one read.
//!
//! git's diff of a commit against the files brings that list up to date and writes it
//! back, under a lock, whatever it is told about locks. Reading must neither write the
//! user's list nor hold its lock while the user's own git wants it, so such a read runs
//! on a copy, in a folder nobody else can enter, that is gone when the read is over.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The copy, and the folder made for it.
pub(crate) struct IndexCopy {
    folder: PathBuf,
}

impl IndexCopy {
    /// Copies the list at `index`. Where there is none yet, as in a repository in which
    /// nothing was ever staged, the copy is none either, which git reads as an empty
    /// list.
    pub(crate) fn of(index: &Path) -> io::Result<IndexCopy> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = std::env::temp_dir();
        let copy = loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let folder = temp.join(format!("yaar-index-{}-{n}", std::process::id()));
            // A name that is taken, by whatever left it there, is passed over: the
            // folder has to be made here, for this user alone.
            match fs::DirBuilder::new().mode(0o700).create(&folder) {
                Ok(()) => break IndexCopy { folder },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        };
        match fs::copy(index, copy.file()) {
            Ok(_) => Ok(copy),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(copy),
            Err(e) => Err(e),
        }
    }

    /// Where the copy is.
    pub(crate) fn file(&self) -> PathBuf {
        self.folder.join("index")
    }
}

impl Drop for IndexCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// The bytes of a worktree's own list of files, taken before a command of the user's own
/// git changes it, so that a command that is refused can put them back.
pub(crate) struct IndexBackup {
    file: PathBuf,
    // None where the list did not exist yet.
    bytes: Option<Vec<u8>>,
}

impl IndexBackup {
    /// Reads the list at `index`; a list that is not there yet is remembered as none.
    pub(crate) fn take(index: &Path) -> io::Result<IndexBackup> {
        let bytes = match fs::read(index) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        Ok(IndexBackup {
            file: index.to_owned(),
            bytes,
        })
    }

    /// Puts the list back as it was, byte for byte: written to a file of its own in the
    /// same folder and renamed over the list, so that no one reads half of it. A list
    /// that did not exist is removed.
    pub(crate) fn restore(self) -> io::Result<()> {
        let Some(bytes) = self.bytes else {
            return match fs::remove_file(&self.file) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        };
        let mut name = self.file.file_name().unwrap_or_default().to_owned();
        name.push(format!(".yaar-restore-{}", std::process::id()));
        let temporary = self.file.with_file_name(name);
        let put_back = fs::write(&temporary, bytes).and_then(|()| {
            // The mode git gave the list, if there is one still.
            if let Ok(old) = fs::metadata(&self.file) {
                fs::set_permissions(&temporary, old.permissions())?;
            }
            fs::rename(&temporary, &self.file)
        });
        if put_back.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        put_back
    }
}
