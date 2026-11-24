//! Removal of the files the Adapters wrote for sessions that no longer run. The Adapters stay
//! locked throughout, so a very large folder delays the agents' reports: the trait gives no way
//! to list the files first.

use std::time::SystemTime;

use crate::agents::Agents;
use crate::lock;
use crate::wiring::Kept;

impl Agents {
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
            // Asked per session as the Adapter comes to it, so a session that began to run
            // meanwhile is spared.
            // A Store that cannot say leaves every session standing.
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
