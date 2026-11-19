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
    // FNV-1a, 64 bits, written out as the Link's socket address is: the hasher of the
    // standard library may change from one compiler to the next, and this value is kept.
    let (present, bytes) = match content {
        None => (0u8, &[][..]),
        Some(bytes) => (1u8, bytes),
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in std::iter::once(&present).chain(bytes) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
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
        ViewedFiles {
            marks: marks.into_iter().collect(),
        }
    }

    /// Marks a file viewed for `content`, its content now, in place of any mark it had.
    pub fn mark(&mut self, path: &Path, content: Option<&[u8]>) {
        self.marks.insert(path.to_owned(), fingerprint(content));
    }

    /// Takes the mark of a file away. A file with no mark is left alone.
    pub fn unmark(&mut self, path: &Path) {
        self.marks.remove(path);
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
    pub fn settle(&mut self, mut now: impl FnMut(&Path) -> Option<Vec<u8>>) -> Vec<PathBuf> {
        let changed: Vec<PathBuf> = self
            .marks
            .iter()
            .filter(|(path, mark)| fingerprint(now(path).as_deref()) != **mark)
            .map(|(path, _)| path.clone())
            .collect();
        for path in &changed {
            self.marks.remove(path);
        }
        changed
    }

    /// The paths marked viewed, ordered by path.
    pub fn viewed(&self) -> Vec<PathBuf> {
        self.marks.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The values of the Python version of the hash, worked out apart from this crate:
    // FNV-1a 64 over the bytes, with the offset 0xcbf29ce484222325 and the prime
    // 0x100000001b3, after the byte 0 (no file) or 1 (a file).
    #[test]
    fn the_fingerprint_is_the_fnv_1a_hash_of_a_byte_for_the_file_and_its_content() {
        assert_eq!(fingerprint(None), "af63bd4c8601b7df");
        assert_eq!(fingerprint(Some(b"")), "af63bc4c8601b62c");
        assert_eq!(fingerprint(Some(b"abc")), "ca907677e91e9e04");
        // Sixteen lowercase digits, zeros kept.
        assert_eq!(fingerprint(Some(b"one")).len(), 16);
    }

    #[test]
    fn an_empty_file_and_no_file_have_different_fingerprints() {
        assert_ne!(fingerprint(None), fingerprint(Some(b"")));
    }

    fn files(marks: &[(&str, &[u8])]) -> ViewedFiles {
        let mut files = ViewedFiles::new();
        for (path, content) in marks {
            files.mark(Path::new(path), Some(content));
        }
        files
    }

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    // REV-18
    #[test]
    fn a_file_loses_its_mark_when_its_content_changes_and_keeps_it_when_another_file_changes() {
        let mut viewed = files(&[("a.rs", b"one"), ("b.rs", b"two")]);
        assert_eq!(viewed.viewed(), paths(&["a.rs", "b.rs"]));

        // `a.rs` has changed.
        let gone = viewed.settle(|path| match path.to_str() {
            Some("a.rs") => Some(b"ONE".to_vec()),
            Some("b.rs") => Some(b"two".to_vec()),
            _ => None,
        });
        assert_eq!(gone, paths(&["a.rs"]));
        assert_eq!(viewed.viewed(), paths(&["b.rs"]));

        // `a.rs` changes again, `b.rs` is as it was: `b.rs` keeps its mark, whatever
        // happened to the other file, and a file with no mark is not given one.
        let gone = viewed.settle(|path| match path.to_str() {
            Some("a.rs") => Some(b"ONE ONE".to_vec()),
            Some("b.rs") => Some(b"two".to_vec()),
            _ => None,
        });
        assert!(gone.is_empty());
        assert_eq!(viewed.viewed(), paths(&["b.rs"]));
    }

    #[test]
    fn a_file_that_is_deleted_loses_its_mark_and_one_marked_again_for_its_new_content_is_viewed() {
        let mut viewed = files(&[("a.rs", b"one"), ("b.rs", b"two")]);
        let gone = viewed.settle(|path| (path == Path::new("b.rs")).then(|| b"two".to_vec()));
        assert_eq!(gone, paths(&["a.rs"]));
        assert_eq!(viewed.viewed(), paths(&["b.rs"]));

        // Marked again for what it holds now.
        viewed.mark(Path::new("a.rs"), Some(b"ONE"));
        let gone = viewed.settle(|path| match path.to_str() {
            Some("a.rs") => Some(b"ONE".to_vec()),
            Some("b.rs") => Some(b"two".to_vec()),
            _ => None,
        });
        assert!(gone.is_empty());
        assert_eq!(viewed.viewed(), paths(&["a.rs", "b.rs"]));
    }

    #[test]
    fn a_mark_given_for_new_content_replaces_the_old_and_an_unmark_takes_it_away() {
        let mut viewed = files(&[("a.rs", b"one")]);
        viewed.mark(Path::new("a.rs"), Some(b"two"));
        let marks: Vec<_> = viewed.marks().collect();
        assert_eq!(
            marks,
            vec![(Path::new("a.rs"), fingerprint(Some(b"two")).as_str())]
        );

        viewed.unmark(Path::new("a.rs"));
        viewed.unmark(Path::new("never-marked.rs"));
        assert!(viewed.viewed().is_empty());
    }

    #[test]
    fn the_marks_as_the_store_keeps_them_come_back_the_same_and_ordered_by_path() {
        let viewed = files(&[("b.rs", b"two"), ("a.rs", b"one")]);
        let kept: Vec<(PathBuf, String)> = viewed
            .marks()
            .map(|(path, mark)| (path.to_owned(), mark.to_owned()))
            .collect();
        assert_eq!(kept[0].0, PathBuf::from("a.rs"));
        assert_eq!(ViewedFiles::from_marks(kept), viewed);
    }

    #[test]
    fn a_file_marked_while_it_was_absent_keeps_its_mark_while_it_stays_absent() {
        let mut viewed = ViewedFiles::new();
        viewed.mark(Path::new("gone.rs"), None);
        assert!(viewed.settle(|_| None).is_empty());
        assert_eq!(viewed.viewed(), paths(&["gone.rs"]));
        assert_eq!(viewed.settle(|_| Some(Vec::new())), paths(&["gone.rs"]));
    }
}
