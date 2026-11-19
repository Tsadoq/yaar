//! The files marked viewed. A mark holds for the content the file had when it was marked:
//! once the content changes, the mark is gone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What is written down of the content of a file when it is marked: 16 lowercase
/// hexadecimal digits of the 64-bit FNV-1a hash of its bytes, after one byte that says
/// whether there is a file. None, where no file is, is a content of its own.
///
/// The value is kept in the Store, so it must not change from one build to the next:
/// the hash is written out here, not taken from the standard library.
pub fn fingerprint(content: Option<&[u8]>) -> String {
    let _ = content;
    todo!()
}

/// The files of one workspace marked viewed, each with the [`fingerprint`] of the content
/// it was marked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewedFiles {
    marks: BTreeMap<PathBuf, String>,
}

impl ViewedFiles {
    /// No file marked.
    pub fn new() -> ViewedFiles {
        ViewedFiles::default()
    }

    /// The marks as the Store keeps them: each path with its fingerprint.
    pub fn from_marks(marks: impl IntoIterator<Item = (PathBuf, String)>) -> ViewedFiles {
        let _ = marks;
        todo!()
    }

    /// Marks a file viewed for `content`, its content now, in place of any mark it had.
    pub fn mark(&mut self, path: &Path, content: Option<&[u8]>) {
        let _ = (path, content);
        todo!()
    }

    /// Takes the mark of a file away. A file with no mark is left alone.
    pub fn unmark(&mut self, path: &Path) {
        let _ = path;
        todo!()
    }

    /// Each mark with its fingerprint, ordered by path.
    pub fn marks(&self) -> impl Iterator<Item = (&Path, &str)> {
        self.marks
            .iter()
            .map(|(path, mark)| (path.as_path(), mark.as_str()))
    }

    /// Takes away the mark of every file whose content changed, reading each marked file's
    /// content now through `now`, and gives the paths whose mark went, ordered by path. A
    /// file whose content is as it was keeps its mark, whatever happened to other files.
    pub fn settle(&mut self, now: impl FnMut(&Path) -> Option<Vec<u8>>) -> Vec<PathBuf> {
        let _ = now;
        todo!()
    }

    /// The paths marked viewed, ordered by path.
    pub fn viewed(&self) -> Vec<PathBuf> {
        self.marks.keys().cloned().collect()
    }
}
