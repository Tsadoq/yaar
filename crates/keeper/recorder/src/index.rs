//! A private copy of git's index for one read: git's diff against the files rewrites the index
//! under a lock whatever it is told, which a read must neither do nor hold.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct IndexCopy {
    folder: PathBuf,
}

impl IndexCopy {
    pub(crate) fn of(index: &Path) -> io::Result<IndexCopy> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = std::env::temp_dir();
        let copy = loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let folder = temp.join(format!("yaar-index-{}-{n}", std::process::id()));
            match fs::DirBuilder::new().mode(0o700).create(&folder) {
                Ok(()) => break IndexCopy { folder },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        };
        // Read and written plainly, not with `fs::copy`: on macOS that asks the system to copy the
        // file, which touches the user's index, and a watcher of the worktree then sees a change at
        // every read.
        let mut from = match fs::File::open(index) {
            Ok(from) => from,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(copy),
            Err(e) => return Err(e),
        };
        let mut to = fs::File::create(copy.file())?;
        io::copy(&mut from, &mut to)?;
        // The index's own time, as `fs::copy` keeps it on macOS: git compares file times with the
        // index's to know which entries it cannot trust, and an index that looked newer would make
        // it trust a stale one.
        to.set_modified(from.metadata()?.modified()?)?;
        Ok(copy)
    }

    pub(crate) fn file(&self) -> PathBuf {
        self.folder.join("index")
    }
}

impl Drop for IndexCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// The bytes of a worktree's index before a command of the user's git changes it, to put back when
/// the command is refused.
pub(crate) struct IndexBackup {
    file: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl IndexBackup {
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

    fn read_now(&self) -> io::Result<Option<Vec<u8>>> {
        match fs::read(&self.file) {
            Ok(now) => Ok(Some(now)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Puts the index back byte for byte through git's own lock (`index.lock` renamed over the
    /// index), so that no one reads half of it and a concurrent git writer is not overwritten.
    /// Where the lock is held the index is left and the error says so.
    pub(crate) fn restore(self) -> io::Result<()> {
        if self.read_now()? == self.bytes {
            return Ok(());
        }
        let mut name = self.file.file_name().unwrap_or_default().to_owned();
        name.push(".lock");
        let lock = self.file.with_file_name(name);
        let mut held = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    io::Error::new(e.kind(), format!("another git holds {}", lock.display()))
                } else {
                    e
                }
            })?;
        // Read with the lock held, so no other git can write between the read and the rename.
        let now = match self.read_now() {
            Ok(now) => now,
            Err(e) => {
                let _ = fs::remove_file(&lock);
                return Err(e);
            }
        };
        if now == self.bytes {
            drop(held);
            return fs::remove_file(&lock);
        }
        let put_back = match &self.bytes {
            Some(bytes) => held.write_all(bytes).and_then(|()| {
                if let Ok(old) = fs::metadata(&self.file) {
                    held.set_permissions(old.permissions())?;
                }
                fs::rename(&lock, &self.file)
            }),
            None => match fs::remove_file(&self.file) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        };
        if self.bytes.is_none() || put_back.is_err() {
            let _ = fs::remove_file(&lock);
        }
        put_back
    }
}
