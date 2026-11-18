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
