//! What the two programs say about the recorded states of the worktrees: how much room
//! they take on disk, and the clean-up that removes what nothing refers to.
//!
//! The Keeper records a state of a worktree at each turn's end, at each send and where a
//! thread needs one, into a state store of the app's own, one store for each workspace.
//! A state that no thread, no round and no reviewed mark refers to is deleted once the set
//! time has passed since it was recorded ([`crate::Setting::KeepStates`]); the turn it was
//! recorded at stays in the history as [`crate::HistoryEntry::ExpiredTurn`]. The user can
//! read the sizes ([`crate::Request::GetStateSizes`]) and clean up at once
//! ([`crate::Request::CleanUpStates`]); every Window is told
//! [`crate::Fact::StatesDeleted`] for each workspace that lost states, whoever caused it.

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::WorkspaceId;

/// The room the state store of one workspace takes on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSize {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The bytes its store takes on disk: the blocks its files use, as `du` counts them.
    pub bytes: u64,
}

/// The room the recorded states take on disk, for each workspace and in total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSizes {
    /// Each workspace the Keeper holds that has a state store, in the order the workspaces
    /// were first kept.
    pub workspaces: Vec<StateSize>,
    /// The bytes the folder of every state store takes on disk: at least the sum of
    /// [`StateSizes::workspaces`], since it also holds the stores of workspaces the Keeper
    /// no longer holds.
    pub total: u64,
}

pub(crate) fn write_state_sizes(out: &mut Out, sizes: &StateSizes) {
    out.list(&sizes.workspaces, |out, size| {
        out.text(size.workspace.as_str());
        out.u64(size.bytes);
    });
    out.u64(sizes.total);
}

pub(crate) fn read_state_sizes(input: &mut In<'_>) -> Result<StateSizes, LinkError> {
    Ok(StateSizes {
        workspaces: input.list(|input| {
            Ok(StateSize {
                workspace: WorkspaceId::new(input.text()?),
                bytes: input.u64()?,
            })
        })?,
        total: input.u64()?,
    })
}
