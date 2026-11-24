use std::collections::BTreeMap;
use std::path::PathBuf;

use yaar_recorder::{Change, ChangedPath};
use yaar_store::{CreditRecord, ReviewId, WorkspaceId};

use crate::engine::{EngineError, Reviews};
use crate::uncommitted::Credited;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedChange {
    pub path: PathBuf,
    pub change: Change,
    /// Sessions credited, in order first credited; empty when no agent made the change.
    pub by: Vec<String>,
    pub shared: bool,
}

impl Reviews {
    /// An empty `credits` marks the state as kept with none (a clean worktree), unlike a state
    /// nothing was kept with.
    pub fn keep_credits(
        &self,
        workspace: &WorkspaceId,
        state: &str,
        credits: &[Credited],
    ) -> Result<(), EngineError> {
        let kept: Vec<CreditRecord> = credits
            .iter()
            .map(|credit| CreditRecord {
                path: credit.path.clone(),
                sessions: credit.by.clone(),
            })
            .collect();
        Ok(self.store.put_credits(workspace, state, &kept)?)
    }

    /// Paths shown to `review`: its session's, those no agent made, or all for the review of no
    /// agent. Credit is the union kept with the states after `from` up to `to`; a state nothing was
    /// kept with counts with the next one kept, even past `to`, an empty keep included.
    pub fn credited_change(
        &self,
        review: &ReviewId,
        from: &str,
        to: &str,
    ) -> Result<Vec<CreditedChange>, EngineError> {
        let review = self.review(review)?;
        let states = self.states_of(review.workspace())?;
        let changed = states.changed_paths(from, to)?;
        let listed = states.states()?;

        let place = |version: &str| listed.iter().position(|state| state.id.as_str() == version);
        let first = place(from).map_or(0, |at| at + 1);
        let last = place(to).unwrap_or(listed.len().saturating_sub(1));

        let mut by: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        let mut waiting = false;
        for (at, state) in listed.iter().enumerate().skip(first) {
            if at > last && !waiting {
                break;
            }
            let kept = self
                .store
                .credits_of(review.workspace(), state.id.as_str())?;
            if kept.is_empty()
                && !self
                    .store
                    .credits_were_kept(review.workspace(), state.id.as_str())?
            {
                waiting |= at <= last;
                continue;
            }
            for credit in kept {
                let sessions = by.entry(credit.path).or_default();
                for session in credit.sessions {
                    if !sessions.contains(&session) {
                        sessions.push(session);
                    }
                }
            }
            waiting = false;
            if at > last {
                break;
            }
        }

        Ok(changed
            .into_iter()
            .filter_map(|ChangedPath { path, change }| {
                let by = by.remove(&path).unwrap_or_default();
                let shown = match review.agent() {
                    None => true,
                    Some(session) => by.is_empty() || by.iter().any(|by| by == session),
                };
                shown.then_some(CreditedChange {
                    shared: by.len() > 1,
                    path,
                    change,
                    by,
                })
            })
            .collect())
    }
}
