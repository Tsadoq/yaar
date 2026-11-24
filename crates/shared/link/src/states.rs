//! Recorded-state sizes and the clean-up of states nothing refers to.

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::WorkspaceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSize {
    pub workspace: WorkspaceId,
    /// Bytes on disk, as `du` counts them (blocks used).
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSizes {
    /// In the order the workspaces were first kept.
    pub workspaces: Vec<StateSize>,
    /// Whole state folder: at least the sum of `workspaces`, as it also holds stores of workspaces
    /// no longer kept.
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
