//! Deleting the states nothing refers to, and the room the stores take on disk.
//!
//! What refers to a state is not the Recorder's to know: the Review engine decides which
//! states and which kept commits go, and hands the Recorder that list
//! ([`StateStore::delete`]). The Recorder deletes those and nothing else, then, when asked,
//! every object no ref of the store reaches any more ([`StateStore::pack`]). A state
//! recorded, or a commit kept, after the engine read what the store holds is in no list it
//! made, and so is never deleted by it.
//!
//! # Deleting
//!
//! [`StateStore::delete`], under the store's lock of writing (the one a recording and a
//! kept commit take), every run of git in the state store with the store's own
//! configuration and the hooks off, as every other run here (`self.run(None, ..)`):
//!
//! 1. `git for-each-ref --format=%(refname)%00%(objectname) refs/yaar/states/ refs/yaar/kept/`
//!    lists what the store names.
//! 2. Every ref of step 1 whose object is in the list handed in is deleted in one
//!    transaction: `git update-ref --stdin` with one line `delete <ref> <object>` each, the
//!    object given, so that a ref moved meanwhile is not deleted. An id of the list that no
//!    ref names is passed over. No ref outside those two folders is ever deleted.
//!
//! It removes no object: until [`StateStore::pack`] runs, the objects stay on disk, read
//! by nobody that asks through a ref.
//!
//! # Packing
//!
//! [`StateStore::pack`], under the store's lock of writing and the guard of the clean-up
//! taken exclusively (below), in this order:
//!
//! 3. `git repack -a -d -q --no-write-bitmap-index`: every object a ref reaches goes into
//!    one new pack and the old packs are removed, so that the objects only deleted states
//!    reached are gone from disk. The store's `shallow` file stops the walk at the commits
//!    a state was taken on, as it does for every read. Not `-A`, not
//!    `--unpack-unreachable`: either would keep unreachable objects loose.
//! 4. `git prune --expire=now`: the loose objects no ref reaches, the trees and blobs a
//!    comparison with the files on disk wrote among them, are removed, and so are the
//!    entries of the `shallow` file that name a commit no longer there.
//! 5. The file `yaar-complete-head`, which names the commit the store last copied whole, is
//!    removed when that commit is no longer held (`git cat-file -e <commit>^{tree}` fails), so
//!    that the next recording copies the commit it is taken on whole again.
//!
//! `--expire=now` is safe only because nothing writes or reads an unnamed object of the store
//! meanwhile: a recording and a kept commit hold the lock of writing, and every call that
//! makes the store hold a version and then runs git on it holds the guard of the clean-up,
//! shared, for the whole call. Those calls are [`StateStore::compare`] (both forms: with
//! the files on disk it writes a tree no ref names), [`StateStore::content_in`],
//! [`StateStore::export`], [`StateStore::changed_paths`], [`StateStore::paths`] and
//! [`StateStore::keep_commit`] (which takes the guard, then the lock of writing). Each takes
//! the guard once, at its start, and calls only inner parts that take none: a guard taken
//! twice on one thread can deadlock once [`StateStore::pack`] waits for it. A comparison
//! that made the store hold a commit with no ref (a commit a state was taken on, found by
//! `cat-file -e`) is thus never pruned between its check and its `diff-tree`.
//!
//! # A reader meanwhile
//!
//! The store's long-running reader of file contents ([`StateStore::content`]) takes no
//! guard and does not wait: it has the packs it opened mapped, and the system keeps a
//! removed file's bytes for whoever holds it open, so a read of a state that stays gives
//! the same bytes before, during and after the clean-up. A pack it had not opened yet when
//! step 3 removed it is looked for again by git itself, which then finds the new pack. A
//! read of a state that was deleted answers that the state holds no such file, as for any
//! state the store does not hold.
//!
//! # The room a store takes
//!
//! [`sizes`] counts what the files under a folder take on disk: for each file and folder,
//! as `symlink_metadata` gives it, the blocks it uses (`st_blocks`, in units of 512 bytes),
//! the way `du` counts, a link counted as itself and never followed. The size of a store is
//! that of its folder; the total is that of the folder that holds every store. An entry
//! directly inside that is a symbolic link (a store a user moved to another disk) is counted
//! as the link's own blocks, listed among the stores by its name, and never followed. The
//! stores hold no hard links (git's fetch and repack write new files), so the walk and `du`
//! agree but for `du`'s rounding to whole units. The walk runs on the caller's thread, never
//! on the thread that draws: in the Keeper it is a request answered apart.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::Path;

use crate::states::{RecordError, StateStore};

/// What [`StateStore::delete`] deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deleted {
    /// The full ids of the states whose refs it deleted, in the order of their numbers.
    pub states: Vec<String>,
    /// The full ids of the kept commits whose refs it deleted, ordered by id.
    pub commits: Vec<String>,
}

/// The room the stores in one folder take on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSizes {
    /// Each folder directly inside, and each symbolic link directly inside (counted as the
    /// link itself), by its name, ordered by name: one state store each. A plain file
    /// directly inside counts in the total only.
    pub stores: Vec<(OsString, u64)>,
    /// What the folder takes on disk, itself and everything in it.
    pub total: u64,
}

impl StateStore {
    /// Deletes the ref of every state, and of every commit kept by
    /// [`StateStore::keep_commit`], whose full id is in `ids`, and nothing else, as the module
    /// doc says; gives what it deleted. Removes no object: [`StateStore::pack`] does.
    ///
    /// The real repository is not read, not written and not needed: the store stands alone.
    pub fn delete(&self, ids: &BTreeSet<String>) -> Result<Deleted, RecordError> {
        let _ = ids;
        todo!("delete the refs of the ids given, as the module doc says")
    }

    /// Removes from disk every object of the store no ref reaches, packing the rest, as the
    /// module doc says: after [`StateStore::delete`] deleted something, or when the user
    /// cleans up at once. Waits for every call that holds the guard of the clean-up.
    pub fn pack(&self) -> Result<(), RecordError> {
        let _ = &self.cleaning;
        todo!("repack, prune and the complete head's file, as the module doc says")
    }
}

/// The room the folder `folder` and everything in it take on disk, and each folder directly
/// inside it, as the module doc says. A folder that is not there takes nothing.
pub fn sizes(folder: &Path) -> io::Result<StoreSizes> {
    let _ = folder;
    todo!("walk the folder and count the blocks, as the module doc says")
}
