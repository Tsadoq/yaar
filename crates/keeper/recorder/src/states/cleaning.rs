//! Deleting the states nothing refers to, and measuring the room the stores take on disk. `delete`
//! removes refs only, only under `refs/yaar/states/` and `refs/yaar/kept/`, and only if a ref still
//! names the object read.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::sync::PoisonError;

use crate::git::{finish_with, unreadable};
use crate::states::{COMPLETE_HEAD, KEPT, RecordError, STATES, StateStore};

/// `st_blocks` counts in units of 512 bytes whatever the file system's block size.
const BLOCK: u64 = 512;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deleted {
    pub states: Vec<String>,
    pub commits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSizes {
    /// Each folder or symbolic link directly inside, by name: one state store each. A plain file
    /// counts in the total only.
    pub stores: Vec<(OsString, u64)>,
    pub total: u64,
}

impl StateStore {
    /// Removes no object: [`StateStore::pack`] does.
    pub fn delete(&self, ids: &BTreeSet<String>) -> Result<Deleted, RecordError> {
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let answer = self.run(
            None,
            [
                "for-each-ref",
                "--format=%(refname)%00%(objectname)",
                STATES,
                KEPT,
            ],
        )?;
        let command = answer.command().to_owned();
        let printed = answer.printed()?;
        let mut states = Vec::new();
        let mut commits = Vec::new();
        let mut transaction = Vec::new();
        for record in printed
            .split(|byte| *byte == b'\n')
            .filter(|record| !record.is_empty())
        {
            let fields: Vec<&[u8]> = record.split(|byte| *byte == 0).collect();
            let [name, object] = fields[..] else {
                return Err(unreadable(
                    &command,
                    format!("the line {:?}", String::from_utf8_lossy(record)),
                )
                .into());
            };
            let (Ok(name), Ok(object)) = (std::str::from_utf8(name), std::str::from_utf8(object))
            else {
                return Err(unreadable(
                    &command,
                    format!("the line {:?}", String::from_utf8_lossy(record)),
                )
                .into());
            };
            if !ids.contains(object) {
                continue;
            }
            // The object given is the one that was read: a ref moved since is not deleted.
            transaction.extend_from_slice(format!("delete {name} {object}\n").as_bytes());
            if name.starts_with(STATES) {
                states.push((number_of(name), object.to_owned()));
            } else {
                commits.push(object.to_owned());
            }
        }
        if transaction.is_empty() {
            return Ok(Deleted::default());
        }
        finish_with(
            self.command_with(None, ["update-ref", "--stdin"]),
            &transaction,
        )?
        .printed()?;
        states.sort();
        commits.sort();
        Ok(Deleted {
            states: states.into_iter().map(|(_, id)| id).collect(),
            commits,
        })
    }

    /// Removes every object of the store no ref reaches, packing the rest.
    pub fn pack(&self) -> Result<(), RecordError> {
        // The guard first, then the lock of writing, in the order `keep_commit` takes them. `prune
        // --expire=now` is safe only because recordings hold the lock of writing and every call
        // that makes the store hold a version and runs git on it holds the guard shared.
        let _exclusive = self
            .cleaning
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let packs = self.packs();
        // Not `-A` or `--unpack-unreachable`, which keep unreachable objects loose.
        self.run(
            None,
            ["repack", "-a", "-d", "-q", "--no-write-bitmap-index"],
        )?
        .printed()?;
        // A failed prune is given after the reader ended, for the repack has removed its packs
        // whatever the prune does.
        let pruned = self
            .run(None, ["prune", "--expire=now", "-v"])
            .and_then(|answer| answer.printed());
        // While the reader lives, a removed pack keeps its blocks, and what `sizes` reports is not
        // what the disk gives back.
        let removed_a_pack = match (packs, self.packs()) {
            (Some(before), Some(after)) => !before.is_subset(&after),
            _ => true,
        };
        if removed_a_pack || pruned.as_ref().is_ok_and(|printed| !printed.is_empty()) {
            self.end_reader();
        }
        pruned?;
        let marker = self.folder.join(COMPLETE_HEAD);
        if let Ok(noted) = fs::read_to_string(&marker) {
            let head = noted.trim();
            let held = !head.is_empty()
                && self
                    .run(None, ["cat-file", "-e", &format!("{head}^{{tree}}")])?
                    .code
                    == Some(0);
            if !held {
                match fs::remove_file(&marker) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => {
                        return Err(RecordError::Store {
                            folder: self.folder.clone(),
                            reason: format!("the note of the complete head cannot be removed: {e}"),
                        });
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn packs(&self) -> Option<BTreeSet<OsString>> {
        fs::read_dir(self.folder.join("objects/pack"))
            .ok()?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .filter(|name| {
                name.as_ref()
                    .map_or(true, |name| name.as_encoded_bytes().ends_with(b".pack"))
            })
            .collect::<Result<_, _>>()
            .ok()
    }

    /// Ends the contents reader, if one runs, under the lock a read takes: a read in flight ends
    /// first, and the next read starts another. The reader takes no guard: a removed pack it has
    /// open stays readable, and git looks again for one it had not opened.
    fn end_reader(&self) {
        let mut kept = self.contents.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(contents) = kept.take() {
            contents.stop();
        }
    }
}

fn number_of(name: &str) -> u64 {
    name.strip_prefix(STATES)
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

pub fn sizes(folder: &Path) -> io::Result<StoreSizes> {
    let top = match fs::metadata(folder) {
        Ok(top) => top,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(StoreSizes {
                stores: Vec::new(),
                total: 0,
            });
        }
        Err(e) => return Err(e),
    };
    let mut total = top.blocks() * BLOCK;
    let mut stores = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let Some(room) = room_of(&entry.path())? else {
            continue;
        };
        total += room;
        let kind = entry.file_type()?;
        if kind.is_dir() || kind.is_symlink() {
            stores.push((entry.file_name(), room));
        }
    }
    stores.sort();
    Ok(StoreSizes { stores, total })
}

/// None when the path is gone by the time it is looked at: git replaces and removes files while it
/// works.
fn room_of(path: &Path) -> io::Result<Option<u64>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut room = meta.blocks() * BLOCK;
    if meta.is_dir() {
        let children = match fs::read_dir(path) {
            Ok(children) => children,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        for child in children {
            let child = match child {
                Ok(child) => child,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            room += room_of(&child.path())?.unwrap_or(0);
        }
    }
    Ok(Some(room))
}
