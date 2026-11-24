//! A version of a worktree written into a folder as plain files.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::git::finish;
use crate::index::IndexCopy;
use crate::states::{RecordError, StateStore};
use crate::worktree::Worktree;

const DRIVER_KEYS: [&str; 3] = ["smudge", "process", "required"];

impl StateStore {
    /// Writes every file of `version` into `into`, which must not exist and whose parent must. No
    /// filter driver runs: each configured driver gets `smudge` and `process` empty and `required`
    /// false. An error removes `into` if this call made it.
    pub fn export(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        into: &Path,
    ) -> Result<(), RecordError> {
        let _guard = self.reading();
        // The folder is claimed first, by making it: a name that is taken is left as it is.
        fs::create_dir(into).map_err(|e| self.store_error(into, &e))?;
        match self.write_into(worktree, version, into) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_dir_all(into);
                Err(e)
            }
        }
    }

    fn write_into(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        into: &Path,
    ) -> Result<(), RecordError> {
        // Comparing the version with itself makes the store hold it, state or commit, and refuses
        // an id that names neither.
        self.compare_held(worktree, version, Some(version))?;

        // An empty index of its own: where none is there to copy, the copy is none, which git reads
        // as empty. The real repository's index is never read.
        let index = IndexCopy::of(&self.folder.join("no-list-of-files"))
            .map_err(crate::git::GitError::from)?;
        let index_file = index.file();
        let scratch = index_file
            .parent()
            .ok_or_else(|| self.store_error(&index_file, &io::Error::other("no folder")))?;

        let mut read = self.command(None);
        read.env("GIT_INDEX_FILE", &index_file)
            .args(["read-tree", version]);
        finish(read)?.printed()?;

        // `checkout-index` wants a work tree, which a bare store has none of: the folder of the
        // index stands in and nothing is written there.
        let mut prefix = OsString::from("--prefix=");
        prefix.push(into);
        prefix.push("/");
        let mut checkout = self.command(None);
        checkout
            .env("GIT_INDEX_FILE", &index_file)
            .env("GIT_WORK_TREE", scratch);
        turn_off_drivers(&mut checkout, &self.drivers()?);
        checkout.args(["checkout-index", "--all"]).arg(prefix);
        finish(checkout)?.printed()?;
        Ok(())
    }

    fn drivers(&self) -> Result<BTreeSet<Vec<u8>>, RecordError> {
        let answer = self.run(
            None,
            [
                "config",
                "--name-only",
                "-z",
                "--get-regexp",
                r"^filter\..*\.(smudge|process|required)$",
            ],
        )?;
        match answer.code {
            Some(0) => Ok(answer
                .stdout
                .split(|byte| *byte == 0)
                .filter_map(|key| key.strip_prefix(b"filter."))
                .filter_map(|rest| {
                    DRIVER_KEYS
                        .iter()
                        .find_map(|key| rest.strip_suffix(format!(".{key}").as_bytes()))
                })
                .map(<[u8]>::to_vec)
                .collect()),
            Some(1) => Ok(BTreeSet::new()),
            _ => Err(answer.failed().into()),
        }
    }

    fn store_error(&self, path: &Path, e: &io::Error) -> RecordError {
        let reason = match e.kind() {
            io::ErrorKind::AlreadyExists => format!("{} exists", path.display()),
            io::ErrorKind::NotFound => format!("the folder above {} is not there", path.display()),
            _ => format!("{} cannot be made: {e}", path.display()),
        };
        RecordError::Store {
            folder: self.folder.clone(),
            reason,
        }
    }
}

/// Turns each driver off through git's environment settings, numbered after those `command`
/// carries or, when it carries none, after those it inherits: a driver's name can hold an
/// equals sign, which `-c` would cut at.
fn turn_off_drivers(command: &mut std::process::Command, drivers: &BTreeSet<Vec<u8>>) {
    let set = command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new("GIT_CONFIG_COUNT"))
        .map(|(_, value)| value.and_then(|value| value.to_str()?.parse().ok()));
    let before: usize = match set {
        Some(count) => count.unwrap_or(0),
        None => std::env::var("GIT_CONFIG_COUNT")
            .ok()
            .and_then(|count| count.parse().ok())
            .unwrap_or(0),
    };
    let mut next = before;
    for driver in drivers {
        for (key, value) in [("smudge", ""), ("process", ""), ("required", "false")] {
            let mut name = b"filter.".to_vec();
            name.extend_from_slice(driver);
            name.push(b'.');
            name.extend_from_slice(key.as_bytes());
            command
                .env(format!("GIT_CONFIG_KEY_{next}"), OsStr::from_bytes(&name))
                .env(format!("GIT_CONFIG_VALUE_{next}"), value);
            next += 1;
        }
    }
    if next > before {
        command.env("GIT_CONFIG_COUNT", next.to_string());
    }
}
