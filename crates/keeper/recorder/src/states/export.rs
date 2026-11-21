//! A version of a worktree written into a folder as plain files: the old side of a diff,
//! which a second language server reads (ADR-016 point 1).

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

/// The keys of a filter driver that make git run a program of the user's on a file.
const DRIVER_KEYS: [&str; 3] = ["smudge", "process", "required"];

impl StateStore {
    /// Writes every file of `version` into the folder `into`, which must not exist and
    /// whose parent must: `version` is the full id of a state of this store or of a commit
    /// of the real repository, which the store copies first through `worktree` as
    /// [`StateStore::compare`] does; with no worktree only what the store holds can be
    /// written.
    ///
    /// The files are written by git's own checkout of a tree into a folder, with an index
    /// of its own made for this call in a temporary file of the system's temporary folder
    /// and removed after: git's `read-tree` of the version into that index, then
    /// `checkout-index --all --prefix=<into>/`, both run in the state store, every hook
    /// off as for every other call here. The store holds the version whole by then, so the
    /// real repository's objects are not read again. No filter driver runs: for each driver
    /// the configuration the store reads defines (`git config --get-regexp
    /// '^filter\..*\.(smudge|process|required)$'`, read as the hooks to turn off are
    /// read), `checkout-index` is given `filter.<driver>.smudge` and
    /// `filter.<driver>.process` empty and `filter.<driver>.required` false, as settings of
    /// its environment, so that a file is written as git stores it (a large-file pointer
    /// stays a pointer) and no driver reaches a server of its own. Line-end conversions of
    /// the attributes apply as in any checkout. A file's mode is kept, an executable one
    /// stays executable; a link is written as a link. The folder holds no `.git` and is no
    /// repository. The real repository gains nothing: no worktree, no ref, no file, no
    /// change of its index; the store gains at most the commit it copied, as a comparison
    /// does.
    ///
    /// A version that is not a full id, that neither the store nor the repository holds,
    /// or a write that fails, is an error, and `into` is removed if it was made; an `into`
    /// that exists, or whose parent does not, is an error and is left as it was.
    pub fn export(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        into: &Path,
    ) -> Result<(), RecordError> {
        // The folder is claimed first, by making it: a name that is taken, by a folder or
        // by anything else, is left as it is, and no one else can write into this one.
        fs::create_dir(into).map_err(|e| self.store_error(into, &e))?;
        match self.write_into(worktree, version, into) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_dir_all(into);
                Err(e)
            }
        }
    }

    /// The steps of [`StateStore::export`] after `into` was made.
    fn write_into(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        into: &Path,
    ) -> Result<(), RecordError> {
        // A comparison of the version with itself is what makes the store hold it, state
        // or commit, and refuses a version that is none; it names no path.
        self.compare(worktree, version, Some(version))?;

        // A list of files of its own, empty, in the system's temporary folder: where none
        // is there to copy, the copy is none, which git reads as an empty list. The real
        // repository's list is not read, let alone written.
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

        // `checkout-index` wants a work tree, which a bare store has none of: the folder
        // of the list stands in, and nothing is written there. The files go to the prefix.
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

    /// The filter drivers the configuration this store reads defines: the names of the
    /// drivers with a key that makes git run a program of the user's.
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
            // git says by its exit code that no key matched.
            Some(1) => Ok(BTreeSet::new()),
            _ => Err(answer.failed().into()),
        }
    }

    /// A failure to make or reach `path`, as an error of the store.
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

/// Gives `command` the settings that turn each driver off, as git's own environment
/// settings, numbered after those the command already carries or, when it carries none,
/// after those this process's environment holds and the command inherits: a driver's name
/// can hold any character, an equals sign too, which `-c` would cut at.
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
