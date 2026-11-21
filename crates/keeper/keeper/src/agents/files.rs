//! The files the Adapters wrote for sessions that no longer run, removed with the clean-up
//! of the recorded states: the reviews too long to ride inline with a message, which an
//! Adapter writes into its own folder of the app for its agent to read.
//!
//! Each Adapter is asked to remove its own (`yaar_agent::Adapter::remove_files_of_ended`),
//! in its folder `<data folder>/agents/<its name>`, with a test that says whether a session
//! still runs: a session the Store holds as running (`Store::running_sessions`), by the
//! CLI's own id of the session. The Keeper names no file of an Adapter: which files are
//! the Adapter's to know.

use crate::agents::Agents;
use crate::wiring::Kept;

impl Agents {
    /// Has every Adapter remove the files it wrote for sessions that no longer run, as the
    /// module doc says, and gives the bytes removed in all. An Adapter that fails is passed
    /// over: what it could not remove stays for the next clean-up.
    pub(crate) fn remove_files_of_ended(&self, kept: &Kept) -> u64 {
        let _ = kept;
        todo!("ask each Adapter, as the module doc says")
    }
}
