//! The files the Adapters wrote for sessions that no longer run, removed with the clean-up
//! of the recorded states: the reviews too long to ride inline with a message, which an
//! Adapter writes into its own folder of the app for its agent to read.
//!
//! Each Adapter is asked to remove its own (`yaar_agent::Adapter::remove_files_of_ended`),
//! in its folder `<data folder>/agents/<its name>`, with a test that says whether a session
//! still runs (a session the Store holds as running, `Store::running_sessions`, by the
//! CLI's own id of the session) and, for the sweep, a time: only files last written before
//! it go, so that a session the user resumes the next day still finds its files. The
//! clean-up at once gives no time. The Keeper names no file of an Adapter: which files are
//! the Adapter's to know.

use std::time::SystemTime;

use crate::agents::Agents;
use crate::lock;
use crate::wiring::Kept;

impl Agents {
    /// Has every Adapter remove the files it wrote for sessions that no longer run, and, with
    /// `written_before`, only those last written before it, as the module doc says; gives the
    /// bytes removed in all. An Adapter that fails is passed over: what it could not remove
    /// stays for the next clean-up.
    pub(crate) fn remove_files_of_ended(
        &self,
        kept: &Kept,
        written_before: Option<SystemTime>,
    ) -> u64 {
        let adapters = lock(&self.core.adapters);
        let mut removed = 0;
        for adapter in adapters.iter() {
            let name = adapter.name();
            let Some(folder) = self.core.folder_of(name) else {
                continue;
            };
            // Asked for each session as the Adapter comes to it, so that a session that
            // began to run since the clean-up began is not taken for one that ended. A
            // Store that cannot say leaves every session standing.
            let runs = |cli: &str| {
                kept.store
                    .session_of_cli(name, cli)
                    .map_or(true, |found| found.is_some_and(|session| session.running))
            };
            match adapter.remove_files_of_ended(folder, &runs, written_before) {
                Ok(bytes) => removed += bytes,
                Err(e) => eprintln!(
                    "yaar-keeper: the files of {} for sessions that ended could not all be removed from {}: {e}",
                    adapter.title(),
                    folder.display()
                ),
            }
        }
        removed
    }
}
