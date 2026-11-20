//! Which servers a row finds, in which order they are tried, and what is shown when none
//! is found (FR-057, FR-060, FR-062, FR-073).
//!
//! The order is FR-060's and the row's ([`order`]): the server the user registered, then
//! the project's own, then one installed on the machine, then one the app downloaded, each
//! kind in the order the row names its servers (ADR-015 point 5). [`order`] is a pure function of
//! what [`look`] found, so that the rule is shown with no file and no process (NAV-07).
//!
//! What "the project's own" is, each server of a row says (`project`: paths under the
//! worktree's root, as `node_modules/@typescript/typescript-{platform}/lib/tsc` or
//! `.venv/bin/ty`); the Rust row names none, since rust-analyzer belongs to the toolchain,
//! whose program rustup puts on the search path and which runs the project's toolchain.

use std::path::{Path, PathBuf};

use crate::port::{Offer, State};
use crate::registry::{Platform, Row};

/// Where a candidate comes from, in the order they are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    /// A command the user registered in their own file.
    Registered,
    /// A path of a server's `project` under the worktree's root.
    Project,
    /// A name of a server's `machine` found in the search path.
    Machine,
    /// A server the app downloaded.
    Downloaded,
}

/// One copy of a server a row may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Where it comes from.
    pub kind: CandidateKind,
    /// Which server of the row it is a copy of, by its place in `Row::servers`: its `args`,
    /// `trial`, `ready` and settings are that server's.
    pub server: usize,
    /// Its program, by its absolute path.
    pub program: PathBuf,
    /// The arguments it is started with: a registered command's own, else its server's
    /// `args`, not yet expanded.
    pub args: Vec<String>,
}

/// What [`look`] found of a row's servers, each kind in the order of the row's servers and,
/// within one server, in the order of its paths or of the search path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// The registered commands, their programs resolved: an absolute path as it is, a bare
    /// name looked for in the search path, a relative path joined to the worktree's root;
    /// one whose program is not there is left out.
    pub registered: Vec<Candidate>,
    /// Each path of a server's `project` that is an executable file.
    pub project: Vec<Candidate>,
    /// Each file named by a server's `machine` that is an executable file of a folder of the
    /// search path: every such folder, in the path's order.
    pub machine: Vec<Candidate>,
    /// The program of the version the app downloaded and uses, for each server that has
    /// one.
    pub downloaded: Vec<Candidate>,
}

/// The candidates of `row`, in the order they are tried (FR-060): every registered one,
/// then every project's own, then every one installed on the machine, then every one the
/// app downloaded, each kind in [`Found`]'s order. A program found twice is tried once, at
/// its first place. A user's row has no downloaded candidate, whatever `found` holds: it
/// replaces the built-in row whole and is never downloaded (FR-073).
pub fn order(row: &Row, found: &Found) -> Vec<Candidate> {
    let _ = (row, found);
    todo!()
}

/// Looks for the candidates of `row`: the registered commands, each server's `project`
/// paths under `root` with `{platform}` expanded, its `machine` names in each folder of
/// `search_path`, and what `servers` holds for it (`crate::download::installed` of the
/// server's name). Reads the file system; runs nothing.
pub fn look(
    row: &Row,
    root: &Path,
    search_path: &[PathBuf],
    servers: &Path,
    platform: Option<Platform>,
) -> Found {
    let _ = (row, root, search_path, servers, platform);
    todo!()
}

/// The offer to download `row`'s server on `platform`: the first server of the row with a
/// download for that platform (`Row::download`); none for a registered row, for a row with
/// no download for that platform, and where the platform is none.
pub fn offer(row: &Row, platform: Option<Platform>) -> Option<Offer> {
    let _ = (row, platform);
    todo!()
}

/// The state shown when no candidate of `row` is left (NAV-06): missing, with
/// [`offer`]'s offer.
pub fn when_none(row: &Row, platform: Option<Platform>) -> State {
    State::Missing {
        offer: offer(row, platform),
    }
}

/// Whether `program`, its links resolved, lies inside one of `worktrees`, the folders of
/// the worktrees of one repository, theirs resolved too (FR-062).
pub fn inside(program: &Path, worktrees: &[&Path]) -> bool {
    let _ = (program, worktrees);
    todo!()
}
