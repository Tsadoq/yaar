//! Which servers run, and when: the workspaces, the one on screen, the search for a
//! server, the question before one inside a repository runs, the trial run, the start, and
//! the stop of idle servers. Downloads and updates are `fetch.rs`'s, which calls the
//! functions this module lists for it.
//!
//! # The servers of a workspace
//!
//! A server is named by its [`ServerKey`]: a workspace, a row and a side. For the
//! workspace on screen, every row whose `markers` lie at the worktree's root has its new
//! side's server ensured ([`ensure`]); any other key is ensured by `answers.rs` at the
//! first ask about a file of its row, and by `answers.rs` for an old side once its export is
//! written. A key ensured goes through these steps, and [`crate::Told::Servers`] is told
//! after each change of any key ([`publish`]):
//!
//! 1. Search: `search::look` and `search::order` give the candidates, kept as the list of
//!    the key's candidates left. Each candidate that lies inside the workspace's repository
//!    (`search::inside`, against the folders of every workspace of that repository) is
//!    passed over unless the repository's answer is yes; with no answer yet the key keeps
//!    the question, for the first such candidate only, and the candidates after it are
//!    tried meanwhile. An answer told later ([`consents`]): a yes searches again
//!    ([`search_again`]) every key of that repository's workspaces that keeps a question or
//!    whose running server came after the one asked about in the order; a no drops the
//!    question, and the key shows what it runs.
//! 2. Trial: the next candidate whose server names `trial` is run once with those arguments,
//!    in the server's environment, on a thread of its own, and passed over unless it ends
//!    with success within [`crate::TRIAL_DEADLINE`] ([`crate::navigator::Event::Trial`]);
//!    one whose server names none goes straight to the start.
//! 3. Start: `driver::Server::start` with the [`Launch`] made from the candidate's server
//!    and its row (the server's `args`, `ready`, `capabilities`, `initialization_options`,
//!    `settings`, `environment`, then for an old side the row's `old_side.environment`; the
//!    row's `extensions` as the language ids; the log `<own cache>/logs/<n>.log`, `<n>`
//!    counting the servers this Navigator started, from 1), every variable expanded:
//!    `{root}` the worktree's folder or the export's, `{worktree}` the worktree's folder,
//!    `{target}` `<cache>/targets/<workspace>/new` or `old`, `{platform}`, `{servers}` the
//!    downloaded version's folder of that server. Ready at [`ServerEvent::Ready`]. A start
//!    that fails, an exit before the server was ready, and an exit after it, are each a
//!    failure of that candidate: the next one is tried (FR-061). `answers::server_gone`
//!    hears of every exit.
//! 4. None left: `search::when_none`, missing with or without the offer.
//!
//! # Where a key stands
//!
//! Each key known is in one of these phases. The state [`publish`] shows for it is the
//! question ([`State::Asking`]) while the key keeps one, whatever its phase; otherwise the
//! one of the table. Every phase but the last counts the key's generation, raised at each
//! search, and an event of an earlier generation is dropped.
//!
//! | Phase | State shown | What moves it on | To |
//! | --- | --- | --- | --- |
//! | trying a candidate | [`State::Starting`] | the trial passed | starting it, the same candidate |
//! | | | the trial failed, or took longer than [`crate::TRIAL_DEADLINE`] | the next candidate left, or none left |
//! | starting a candidate | [`State::Starting`] | [`ServerEvent::Ready`] | ready |
//! | | | the start failed, or [`ServerEvent::Exited`] | the next candidate left, or none left |
//! | ready | [`State::Ready`], with `update` when the program is a downloaded one older than the row pins | [`ServerEvent::Exited`] | the next candidate left, or none left |
//! | none left | `search::when_none` | [`search_again`] (a yes, or `fetch.rs`) | a new search |
//! | downloading | [`State::Downloading`] | [`search_again`], from `fetch.rs` | a new search |
//!
//! "The next candidate left": the head of the key's list is taken off it; one inside the
//! repository is skipped as step 1 says; one whose server names `trial` is tried, any other
//! started. An empty list is "none left". Any phase goes when the key is stopped
//! ([`stop`]): its server stopped, the key forgotten.
//!
//! # Stopping
//!
//! The time a workspace left the screen is read from the [`crate::Clock`] when it leaves.
//! [`look_at_the_clock`], run before every event is handled and at least every
//! [`crate::LOOK_EVERY`], stops every server of each workspace off screen since
//! [`crate::NavigatorConfig::stop_after`] or longer, both sides, and forgets their keys:
//! they are not listed in [`crate::Told::Servers`] any more. A workspace that comes back
//! on screen has its new side's servers ensured again; an old side's server starts again
//! at the next ask about the old side. A workspace that leaves the list of
//! [`crate::Ask::Workspaces`] has every server stopped at once, and its build folders,
//! `<cache>/targets/<workspace>`, removed. [`close`] stops every server.
//!
//! Every stop here is done before the function that does it returns: [`stop`] calls
//! `Server::stop` with [`crate::STOP_DEADLINE`], which returns once the process is reaped,
//! and only then forgets the key and publishes. So [`look_at_the_clock`], [`workspaces`],
//! [`close`], and `old_side.rs` through [`stop`], have every process they stop gone when
//! they return, and a [`crate::Told::Servers`] told after them names no process that
//! still runs. Nothing here waits for a server's answer or its readiness: those come as
//! events.
//!
//! # What other modules call
//!
//! `answers.rs` and `old_side.rs` call [`ensure`], [`ready_server`], [`state_of`],
//! [`stop`] and [`publish`]; `fetch.rs` calls [`offering`], [`updating`], [`downloading`],
//! [`search_again`] and [`publish`]; nothing else here.

use std::path::Path;

use yaar_link::WorkspaceId;

use crate::driver::{Launch, Server, ServerEvent};
use crate::navigator::{Core, ServerKey};
use crate::port::{Offer, RepositoryConsent, State, WorkspaceRoot};
use crate::search::Candidate;

/// What this module keeps in [`Core::life`]: every key ensured with where it stands, the
/// servers that run, the times workspaces left the screen.
#[derive(Default)]
pub(crate) struct Life {}

/// At the start of the Navigator: reads the registry from
/// [`crate::NavigatorConfig::builtin`] and the user's file into [`Core::registry`],
/// writing each [`crate::registry::RowError`] to the standard error as one line; tells
/// [`crate::Told::Languages`], each row's name, title and extensions in the registry's
/// order; and removes every folder `<cache>/<pid>-<k>` whose `<pid>`, the part of its name
/// before the `-`, names no process that runs (a folder whose name is not of that form is
/// left as it is).
pub(crate) fn started(core: &mut Core) {
    let _ = core;
    todo!()
}

/// [`crate::Ask::Workspaces`]: keeps the list; every key of a workspace no longer listed, or
/// whose worktree is gone, is stopped and forgotten; a workspace listed before and no
/// longer listed has `<cache>/targets/<workspace>` removed.
pub(crate) fn workspaces(core: &mut Core, roots: Vec<WorkspaceRoot>) {
    let _ = (core, roots);
    todo!()
}

/// [`crate::Ask::OnScreen`]: the workspace that left the screen has the time noted; the one that
/// came on screen has the time taken away and its new side's servers ensured, by markers.
/// It ends with [`publish`] every time, also when nothing changed: what it tells is what a
/// test that moved its clock reads after the clock was looked at.
pub(crate) fn on_screen(core: &mut Core, workspace: Option<WorkspaceId>) {
    let _ = (core, workspace);
    todo!()
}

/// [`crate::Ask::Consents`]: keeps the answers, and searches again as the module doc says.
pub(crate) fn consents(core: &mut Core, consents: Vec<RepositoryConsent>) {
    let _ = (core, consents);
    todo!()
}

/// What a server's reading thread told: ready, an answer (handed to
/// `answers::server_answered`), or its exit (handed to `answers::server_gone`, then the
/// next candidate is tried). An event of an earlier generation of the key is dropped.
pub(crate) fn server_event(core: &mut Core, key: ServerKey, generation: u64, event: ServerEvent) {
    let _ = (core, key, generation, event);
    todo!()
}

/// A trial run ended: the candidate is started, or passed over for the next.
pub(crate) fn trial_ended(
    core: &mut Core,
    key: ServerKey,
    generation: u64,
    candidate: Candidate,
    passed: bool,
) {
    let _ = (core, key, generation, candidate, passed);
    todo!()
}

/// Stops the servers of every workspace off screen for the set time or longer, as the
/// module doc says.
pub(crate) fn look_at_the_clock(core: &mut Core) {
    let _ = core;
    todo!()
}

/// At the end: stops every server, each as [`stop`] does.
pub(crate) fn close(core: &mut Core) {
    let _ = core;
    todo!()
}

/// Makes sure the server of `key` is running or being looked for, `root` the folder it
/// serves: the worktree's folder for [`crate::navigator::Side::New`], the export's for an
/// old side. Starts the search when the key is not known yet. Gives where it stands now.
pub(crate) fn ensure(core: &mut Core, key: &ServerKey, root: &Path) -> State {
    let _ = (core, key, root);
    todo!()
}

/// The server of `key`, when one runs and is ready.
pub(crate) fn ready_server<'a>(core: &'a Core, key: &ServerKey) -> Option<&'a Server> {
    let _ = (core, key);
    todo!()
}

/// Where the server of `key` stands; none for a key not known.
pub(crate) fn state_of(core: &Core, key: &ServerKey) -> Option<State> {
    let _ = (core, key);
    todo!()
}

/// Stops the server of `key`, if one runs, with `Server::stop` and
/// [`crate::STOP_DEADLINE`], which returns once its process is reaped; then forgets the key
/// and publishes.
pub(crate) fn stop(core: &mut Core, key: &ServerKey) {
    let _ = (core, key);
    todo!()
}

/// The keys of the row `language` whose state is missing with an offer, each with it.
pub(crate) fn offering(core: &Core, language: &str) -> Vec<(ServerKey, Offer)> {
    let _ = (core, language);
    todo!()
}

/// The keys of the row `language` whose state shows an update, each with it.
pub(crate) fn updating(core: &Core, language: &str) -> Vec<(ServerKey, Offer)> {
    let _ = (core, language);
    todo!()
}

/// Shows `key` as [`State::Downloading`] with `offer` until it is searched again, and
/// publishes.
pub(crate) fn downloading(core: &mut Core, key: &ServerKey, offer: Offer) {
    let _ = (core, key, offer);
    todo!()
}

/// Searches again for `key`, from step 1 of the module doc, with the root it was ensured
/// with: stops its server if one runs (as [`stop`] does, without forgetting the key),
/// raises its generation, and publishes. A key not known is left alone.
pub(crate) fn search_again(core: &mut Core, key: &ServerKey) {
    let _ = (core, key);
    todo!()
}

/// Tells [`crate::Told::Servers`]: every key known, in the order the port's doc says.
pub(crate) fn publish(core: &Core) {
    let _ = core;
    todo!()
}

/// The [`Launch`] of `candidate` for `key`, every variable of its row expanded; an error
/// for a variable the row names that has no value.
pub(crate) fn launch(
    core: &Core,
    key: &ServerKey,
    root: &Path,
    candidate: &Candidate,
) -> Result<Launch, String> {
    let _ = (core, key, root, candidate);
    todo!()
}
