//! Carrying every thread of a workspace to each new state the engine records, one state
//! at a time, and between any two trees: commits and recorded states alike, which only
//! the Recorder tells apart. Built by the engine's package "following through rounds" of
//! slice 6.

use yaar_recorder::StateId;
use yaar_store::{ThreadId, WorkspaceId};

use crate::engine::{EngineError, Reviews};
use crate::follow::Placed;

impl Reviews {
    /// Carries every thread of every review of `workspace` that is not resolved and sits on
    /// the new side to the recorded state `to`, from the tree it sits in now (where it was
    /// carried last, else its anchor's), by [`crate::follow::carry_file`] over the files of
    /// the two trees as the workspace's state store reads them and pairs their renames;
    /// keeps where each one sits now. Gives the threads whose place or notice changed.
    ///
    /// A thread is carried only forward: to a `to` newer than the tree it sits in, by the
    /// order of the state store's list (`yaar_recorder::StateStore::states`), whose order
    /// is the order the states were recorded. A thread that sits in `to` already, or in a
    /// state newer than `to`, is left: two recordings of one workspace can run at once (a
    /// turn's end and a new thread's first save, say), and their calls of this may come in
    /// the other order. A thread that sits on its anchor, in a commit, is carried to any
    /// state.
    ///
    /// The engine's turn is not held while the trees are read and diffed, which may take
    /// long with many threads: a report of the agent, which waits for the engine's turn,
    /// must not wait for that. The turn is taken only at the end, to read the threads
    /// again, leave out each one whose place changed meanwhile (another call carried it,
    /// or the reviewer resolved it), and keep the rest with one `Store::set_places`.
    pub fn follow(
        &self,
        workspace: &WorkspaceId,
        to: &StateId,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let _ = (workspace, to);
        todo!()
    }

    /// Where a thread placed at `placed` in tree `from` sits in tree `to`, two versions of
    /// `workspace` read through its state store: recorded states or commits it holds,
    /// whichever each is. Nothing is kept.
    pub fn carry_between(
        &self,
        workspace: &WorkspaceId,
        placed: &Placed,
        from: &str,
        to: &str,
    ) -> Result<Placed, EngineError> {
        let _ = (workspace, placed, from, to);
        todo!()
    }

    /// The engine took the state `state` of `workspace`: every thread is carried to it, by
    /// [`Reviews::follow`], and a failure is written down, since the state was taken all
    /// the same. Called after the state was recorded, without the engine's turn. Does
    /// nothing until the package that builds following fills it in, so that every
    /// recording of slices 3 to 5 runs as it did.
    pub(crate) fn followed(&self, workspace: &WorkspaceId, state: &StateId) {
        let _ = (workspace, state);
    }
}
