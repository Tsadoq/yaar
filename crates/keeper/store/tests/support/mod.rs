//! What the component tests of the Store share.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_store::FILE_NAME;

/// A folder of the test's own, removed when the test ends, whether it passes or fails.
pub struct Scratch(pub PathBuf);

impl Scratch {
    /// A new, empty folder under the system's temporary folder.
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!("yaar-store-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("make the scratch folder");
        Self(folder)
    }

    /// The database file of this folder.
    pub fn file(&self) -> PathBuf {
        self.0.join(FILE_NAME)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
