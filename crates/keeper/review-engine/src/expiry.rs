//! The clean-up of recorded states: which states the engine keeps, and when it has the
//! Recorder delete the others (FR-125, FR-126).
//!
//! # What refers to a state
//!
//! A state, or a commit the state store keeps, is referred to, and kept whatever its age,
//! when its full id is one of these, as the Store holds them for the workspace:
//!
//! | Who refers | Where the Store keeps it |
//! | --- | --- |
//! | a thread, where it was written | `Thread::anchor.tree` of every thread of every review of the workspace |
//! | a thread, where it sits now | `Thread::now`'s `ThreadPlace::tree` |
//! | a round | `Round::state` of every round |
//! | a reviewed mark | `ReviewRecord::reviewed` of every review |
//! | a turn that has not ended | `TurnRecord::start` of every turn of the workspace's sessions whose `ended` is none |
//! | the worktree itself | the newest state of the store, while the workspace's worktree is there |
//!
//! The last two are not FR-125's three; they are what a running turn counts its change from
//! and what the next turn will count from, and the live diff compares the files on disk
//! with (the Keeper takes a turn's start as the newest state when the turn starts). A state
//! deleted under them would leave a running turn, or the next, with no start. A worktree
//! that is gone has no next turn: its newest state is not kept for it.
//!
//! # When a state is deleted
//!
//! - [`crate::Reviews::expire`], by the set time (FR-125): a state no one refers to is
//!   deleted once `keep_for` has passed since it was recorded, as
//!   `yaar_recorder::StateStore::recorded_times` gives it and the engine's clock reads now
//!   ([`crate::Reviews::with_clock`]). A state recorded at a turn's end is that turn's: the
//!   turn stays in the Store, its `state` still naming the deleted id, and the history shows
//!   it as expired ([`TurnMark::Expired`]). A kept commit no thread, round or mark names is
//!   let go whatever its age: it was kept for a reader, and is copied again when one needs
//!   it.
//! - [`crate::Reviews::clean_up`], at once (FR-126): every state and kept commit no one
//!   refers to, whatever its age.
//! - A workspace whose worktree is gone: by the set time, every state recorded longer ago
//!   than `keep_for` is deleted whether or not a thread refers to it (a thread keeps its text,
//!   and a gone worktree's reviews are text). Once its store holds no state and no kept
//!   commit, the store's folder is removed (`<states>/<workspace>.git`) and forgotten by the
//!   engine. The clean-up at once does the same for a gone worktree's states nothing refers
//!   to.
//!
//! Both run under the engine's turn of writing, so that no thread, round or mark made while
//! they decide is missed: the list of what is kept is read, and the Recorder's clean-up
//! made, in that one turn. The Recorder's clean-up holds the state store's own lock of
//! writing, so no recording of the same store runs meanwhile either.
//!
//! # The room the states take
//!
//! [`crate::Reviews::state_sizes`] gives `yaar_recorder::sizes` of the folder of the state
//! stores, each store's folder mapped back to its workspace by its name, `<workspace>.git`;
//! a folder of another name counts in the total only.

use std::collections::BTreeMap;
use std::time::Duration;

use yaar_store::TurnId;

use crate::WorkspaceId;
use crate::engine::{EngineError, Reviews};

/// The time a state nothing refers to is kept when the user set none: 30 days (FR-125).
pub const KEEP_STATES: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What one clean-up of a workspace deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expired {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The full ids of the states deleted.
    pub states: Vec<String>,
    /// The room its store took on disk before, and after; after is 0 when the store's
    /// folder was removed.
    pub before: u64,
    /// See `before`.
    pub after: u64,
}

/// How a turn of a workspace's history stands with its recorded state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMark {
    /// Its state is in the store: the turn can be shown.
    Held,
    /// Its state was recorded and deleted since by a clean-up: the turn is expired.
    Expired,
    /// No state was recorded at its end: the recording failed, or it has not ended.
    NotRecorded,
}

/// The room the recorded states take on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSizes {
    /// Each workspace with a state store, by the name of the store's folder, in the order
    /// of their names.
    pub workspaces: Vec<(WorkspaceId, u64)>,
    /// What the folder of every state store takes.
    pub total: u64,
}

impl Reviews {
    /// Deletes the states of `workspace` that no one refers to and that were recorded at
    /// least `keep_for` ago, by the engine's clock, as the module doc says; `worktree_gone`
    /// for a workspace whose worktree is gone, whose states go by the time alone. Gives what
    /// it deleted; a workspace with no state store deletes nothing and makes none.
    pub fn expire(
        &self,
        workspace: &WorkspaceId,
        keep_for: Duration,
        worktree_gone: bool,
    ) -> Result<Expired, EngineError> {
        let _ = (workspace, keep_for, worktree_gone);
        todo!("delete what the set time and the references allow, as the module doc says")
    }

    /// Deletes at once every state of `workspace` that no one refers to, whatever its age,
    /// as the module doc says.
    pub fn clean_up(
        &self,
        workspace: &WorkspaceId,
        worktree_gone: bool,
    ) -> Result<Expired, EngineError> {
        let _ = (workspace, worktree_gone);
        todo!("delete what no one refers to, as the module doc says")
    }

    /// How each turn of `workspace` stands with its recorded state, by the turn's id: every
    /// turn of the workspace's sessions the Store holds.
    pub fn turn_marks(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<BTreeMap<TurnId, TurnMark>, EngineError> {
        let _ = workspace;
        todo!("held, expired or not recorded, for each turn")
    }

    /// The room the recorded states take on disk, for each workspace and in total, as the
    /// module doc says.
    pub fn state_sizes(&self) -> Result<StateSizes, EngineError> {
        todo!("the sizes of the folder of the state stores")
    }
}
