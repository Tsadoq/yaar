//! Who is credited with each path, kept with each recorded state, and what one review shows
//! of a change between two versions: the paths of its session and those no agent made, a
//! path two sessions changed whole in both reviews, marked. Built by the engine's package
//! "credits with recorded states" of slice 6.

use std::collections::BTreeMap;
use std::path::PathBuf;

use yaar_recorder::{Change, ChangedPath};
use yaar_store::{CreditRecord, ReviewId, WorkspaceId};

use crate::engine::{EngineError, Reviews};
use crate::uncommitted::Credited;

/// One path of a change between two versions as a review shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedChange {
    /// The path in the newer version, relative to the folder of the worktree.
    pub path: PathBuf,
    /// What happened to it between the two versions.
    pub change: Change,
    /// The sessions credited with it in any state after the older version up to the newer
    /// one, by their ids, in the order first credited; empty for a change no agent made.
    pub by: Vec<String>,
    /// Whether two sessions or more are credited with it: the review shows the file whole
    /// all the same, and marks it. For the review of no agent too, which shows every path:
    /// a path of one session alone is not marked there.
    pub shared: bool,
}

impl Reviews {
    /// Keeps, with the recorded state `state` of `workspace`, who is credited with each
    /// path in `credits`: every uncommitted change of the worktree, as Workspaces credited
    /// them once the state was recorded. What was kept with that state before is replaced.
    /// The Keeper calls it at each turn's end; the other states the engine records (at a
    /// send, a mark of reviewed, a new thread's first save, a worktree's first watch) keep
    /// none, and [`Reviews::credited_change`] says how they count.
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

    /// The paths that differ from version `from` to the recorded state `to` that `review`
    /// shows, in the order of the paths: those credited to its session, those no agent
    /// made, and for the review of no agent every one. A path credited to its session and
    /// to another is in it whole, with [`CreditedChange::shared`]; the engine never divides
    /// the changes of one file.
    ///
    /// Who is credited with a path is the union, in the order first credited, of what was
    /// kept for it with each state recorded after `from` up to `to`, in the order of the
    /// state store's list (`yaar_recorder::StateStore::states`). A state with no credits
    /// kept, as one recorded at a send between two turn ends, counts with the first state
    /// after it, even past `to`, that has credits kept: the turn end that follows a send
    /// keeps the credits of the writes before the send too. A path none of those names is
    /// one no agent made. A version that is no state of the list, as a commit kept in the
    /// store, counts from the first state when it is `from`, and to the newest when it is
    /// `to`.
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

        // The states after `from` up to `to`, as places in the list: a version that is not a
        // state, as a commit kept in the store, is no place, and counts from the start for
        // `from` and to the end for `to`.
        let place = |version: &str| listed.iter().position(|state| state.id.as_str() == version);
        let first = place(from).map_or(0, |at| at + 1);
        let last = place(to).unwrap_or(listed.len().saturating_sub(1));

        let mut by: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        // The states in range with no credits kept, which count with the next state that
        // has some.
        let mut waiting = false;
        for (at, state) in listed.iter().enumerate().skip(first) {
            if at > last && !waiting {
                break;
            }
            let kept = self
                .store
                .credits_of(review.workspace(), state.id.as_str())?;
            if kept.is_empty() {
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
