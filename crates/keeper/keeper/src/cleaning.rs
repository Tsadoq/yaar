//! The clean-up of the recorded states, between the Link, the Review engine and the
//! Adapters: the sizes a Window asks for, the clean-up the user asks for, and the one the
//! set time makes, at the Keeper's start and every [`SWEEP_EVERY`] after.
//!
//! | Request | Answered by | Facts |
//! | --- | --- | --- |
//! | `GetStateSizes` | `Reviews::state_sizes`, each store mapped to a workspace the Store holds, in the order the workspaces were first kept; a store of no workspace the Store holds counts in the total only | none |
//! | `CleanUpStates` | for every workspace the Store holds, `Reviews::clean_up`, the worktree gone or not as `Workspaces::worktree` finds it; then `Agents::remove_files_of_ended`; then the sizes, as `GetStateSizes` | `StatesDeleted` of each workspace that lost a state, after the reply |
//!
//! Both are answered apart, on a thread of their own: a clean-up repacks every store, and a
//! size walks every folder of states.
//!
//! # The sweep by the set time
//!
//! [`Sweeps::start`] starts one thread at the Keeper's start. It sweeps at once, then every
//! [`SWEEP_EVERY`], until the Keeper ends ([`Sweeps`] is dropped, which wakes and ends the
//! thread: it waits on a channel with a timeout, never in a plain sleep). A sweep:
//!
//! 1. reads the set time from the Store's settings, `KeepStates` in days, or
//!    `yaar_review_engine::KEEP_STATES` when the user set none;
//! 2. for every workspace the Store holds, calls `Reviews::expire` with that time, the
//!    worktree gone or not as `Workspaces::worktree` finds it, under the Keeper's parts read
//!    as a request reads them (the parts may be gone: the sweep then ends);
//! 3. has the Adapters remove the files of sessions that no longer run;
//! 4. tells every Window `StatesDeleted` of each workspace that lost a state.
//!
//! The age of a state is the engine's clock's, which the Keeper was started with
//! (`run_with_clock`); [`SWEEP_EVERY`] is only how often it looks, real time. A test of the
//! sweep starts a Keeper of its own with a clock it moved past the set time: the sweep at
//! the start is the one it reads, and it waits for that sweep's `StatesDeleted`, which is
//! told to the Windows attached by then, or for `GetHistory` to show the turn expired.
//! A failure of one workspace's sweep is written to the standard error in one line,
//! starting `yaar-keeper:`, and the others go on.

use std::sync::Weak;
use std::time::Duration;

use yaar_link::{Fact, Refusal, Reply};

use crate::Shared;
use crate::agents::Agents;
use crate::wiring::Kept;

/// How often the sweep by the set time looks, after the one at the Keeper's start.
pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// `Request::GetStateSizes`, as the module doc says.
pub(crate) fn sizes(kept: &Kept) -> Result<Reply, Refusal> {
    let _ = kept;
    todo!("the sizes, as the module doc says")
}

/// `Request::CleanUpStates`, as the module doc says: puts `StatesDeleted` of each workspace
/// that lost a state into `facts`.
pub(crate) fn clean_up_now(
    kept: &Kept,
    agents: &Agents,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, agents, facts);
    todo!("clean every workspace up, then the Adapters' files, as the module doc says")
}

/// The thread of the sweep by the set time, while the Keeper runs. Dropping it ends the
/// thread and waits for it.
pub(crate) struct Sweeps {
    _shared: Weak<Shared>,
}

impl Sweeps {
    /// Starts the sweeps of the Keeper whose shared state is `shared`, as the module doc
    /// says.
    pub(crate) fn start(shared: Weak<Shared>) -> Sweeps {
        // Until the sweep is built the Keeper sweeps nothing.
        Sweeps { _shared: shared }
    }
}
