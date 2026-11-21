//! Deleting the states nothing refers to, and the room the stores take on disk.
//!
//! What refers to a state is not the Recorder's to know: the Review engine decides which
//! states and which kept commits stay, and hands the Recorder that list
//! ([`StateStore::clean_up`]). The Recorder deletes everything else the store names, and
//! then every object no ref of the store reaches any more.
//!
//! # How a store is cleaned
//!
//! Under the store's lock of writing, the one a recording and a kept commit take, so that
//! no recording, no copy of a commit and no comparison with the files on disk runs at the
//! same time, in this order, every run of git in the state store with the store's own
//! configuration and the hooks off, as every other run here:
//!
//! 1. `git for-each-ref --format=%(refname)%00%(objectname) refs/yaar/states/ refs/yaar/kept/`
//!    lists what the store names.
//! 2. Every ref whose object is not in `keep` is deleted in one transaction:
//!    `git update-ref --stdin` with one line `delete <ref> <object>` each, the object given
//!    so that a ref moved meanwhile is not deleted. Nothing to delete: steps 3 to 5 still
//!    run, since a comparison may have left objects no ref names.
//! 3. `git repack -a -d -q --no-write-bitmap-index`: every object a ref reaches goes into
//!    one new pack and the old packs are removed, so that the objects only deleted states
//!    reached are gone from disk. The store's `shallow` file stops the walk at the commits
//!    a state was taken on, as it does for every read.
//! 4. `git prune --expire=now`: the loose objects no ref reaches, the trees and blobs a
//!    comparison with the files on disk wrote among them, are removed, and so are the
//!    entries of the `shallow` file that name a commit no longer there.
//! 5. The file `yaar-complete-head`, which names the commit the store last copied whole, is
//!    removed when that commit is no longer held (`git cat-file -e <commit>^{tree}` fails), so
//!    that the next recording copies the commit it is taken on whole again.
//!
//! The lock of writing is held from step 1 to step 5: `--expire=now` is safe only because
//! nothing writes an object into the store meanwhile. That is why a comparison with the
//! files on disk ([`StateStore::compare`] with `to` none) takes the lock of writing too,
//! from the moment it writes the tree of the files on disk until git has compared it: the
//! tree and its blobs are named by no ref, and step 4 would take them from under it.
//!
//! # A reader meanwhile
//!
//! The store's long-running reader of file contents is not stopped and does not wait: it
//! has the packs it opened mapped, and the system keeps a removed file's bytes for whoever
//! holds it open, so a read of a state that stays gives the same bytes before, during and
//! after the clean-up. A pack it had not opened yet when step 3 removed it is looked for
//! again by git itself, which then finds the new pack. A read of a state that was deleted
//! answers that the state holds no such file, as for any state the store does not hold.
//!
//! # The room a store takes
//!
//! [`sizes`] counts what the files under a folder take on disk: for each file and folder,
//! as `symlink_metadata` gives it, the blocks it uses (`st_blocks`, in units of 512 bytes),
//! the way `du` counts, a link counted as itself and never followed. The size of a store is
//! that of its folder; the total is that of the folder that holds every store. The walk
//! runs on the caller's thread, never on the thread that draws: in the Keeper it is a
//! request answered apart.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::Path;

use crate::states::{RecordError, StateStore};

/// What [`StateStore::clean_up`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    /// The full ids of the states it deleted, in the order of their numbers.
    pub states: Vec<String>,
    /// The full ids of the kept commits it let go.
    pub commits: Vec<String>,
    /// What the store took on disk before, as [`sizes`] counts.
    pub before: u64,
    /// What it takes after.
    pub after: u64,
}

/// The room the stores in one folder take on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSizes {
    /// Each folder directly inside, by its name, ordered by name: one state store each.
    /// A plain file directly inside counts in the total only.
    pub stores: Vec<(OsString, u64)>,
    /// What the folder takes on disk, itself and everything in it.
    pub total: u64,
}

impl StateStore {
    /// Deletes every state, and every commit kept by [`StateStore::keep_commit`], whose full
    /// id is not in `keep`, then every object of the store no ref reaches any more: the
    /// module doc says how, and what a reader meanwhile sees. Gives what it deleted and the
    /// room the store took before and after.
    ///
    /// The real repository is not read, not written and not needed: the store stands alone.
    /// An id in `keep` that the store does not hold is passed over.
    pub fn clean_up(&self, keep: &BTreeSet<String>) -> Result<Cleaned, RecordError> {
        let _ = keep;
        todo!("delete what keep does not name, repack and prune, as the module doc says")
    }
}

/// The room the folder `folder` and everything in it take on disk, and each folder directly
/// inside it, as the module doc says. A folder that is not there takes nothing.
pub fn sizes(folder: &Path) -> io::Result<StoreSizes> {
    let _ = folder;
    todo!("walk the folder and count the blocks, as the module doc says")
}
