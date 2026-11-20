//! Which servers run, and when: the workspaces, the one on screen, the search for a
//! server, the question before one inside a repository runs, the trial run, the start,
//! the downloads and updates the user asked for, and the stop of idle servers.
//!
//! # The servers of a workspace
//!
//! A server is named by its [`ServerKey`]: a workspace, a row and a side. For the
//! workspace on screen, every row whose `markers` lie at the worktree's root has its new
//! side's server ensured ([`ensure`]); any other key is ensured by `answers.rs` at the
//! first ask about a file of its row, and by `old_side.rs` at the first ask about the old
//! side. A key ensured goes through these steps, and [`crate::Told::Servers`] is told after
//! each change of any key ([`publish`]):
//!
//! 1. Search: `search::look` and `search::order` give the candidates. Each candidate that
//!    lies inside the workspace's repository (`search::inside`, against the folders of
//!    every workspace of that repository) is passed over unless the repository's answer is
//!    yes; with no answer yet the state is [`State::Asking`] for the first such candidate,
//!    and the candidates after it are tried meanwhile. An answer told later
//!    ([`consents`]): a yes searches again every key of that repository's workspaces whose
//!    state is asking or whose running server came after the one asked about; a no leaves
//!    the question gone.
//! 2. Trial: the next candidate whose server names `trial` is run once with those arguments,
//!    in the server's environment, on a thread of its own, and passed over unless it ends
//!    with success within [`crate::TRIAL_DEADLINE`] ([`crate::navigator::Event::Trial`]);
//!    one whose server names none goes straight to the start.
//! 3. Start: `driver::Server::start` with the [`Launch`] made from the candidate's server
//!    and its row (the server's `args`, `ready`, `capabilities`, `initialization_options`,
//!    `settings`, `environment`, then for an old side the row's `old_side.environment`; the
//!    row's `extensions` as the language ids), every variable expanded: `{root}` the worktree's folder or the export's, `{worktree}` the worktree's
//!    folder, `{target}` `<cache>/targets/<workspace>/new` or `old`, `{platform}`,
//!    `{servers}` the downloaded version's folder of that server. The state is [`State::Starting`], then
//!    [`State::Ready`] at [`ServerEvent::Ready`]. A start that fails, an exit before the
//!    server was ready, and an exit after it, are each a failure of that candidate: the
//!    next one is tried (FR-061). `answers::server_gone` hears of every exit.
//! 4. None left: `search::when_none`, missing with or without the offer.
//!
//! A download ([`download`]) is fetched only for a row whose state shows the offer, the
//! offer's server's download for this machine (`Row::download`), on a
//! thread of its own ([`crate::navigator::Event::Downloaded`]), with the state
//! [`State::Downloading`] meanwhile; the version installed then is started, and a failed
//! download goes back to missing with its offer, the reason written to the standard
//! error. An update ([`update`]) is the same for a row whose running server is the
//! downloaded one and whose `update` an older version shows; nothing else ever downloads
//! (FR-063, FR-073).
//!
//! # Stopping
//!
//! The time a workspace left the screen is read from the [`crate::Clock`] when it leaves.
//! [`look_at_the_clock`], run before every event is handled and at least every
//! [`crate::LOOK_EVERY`], stops every server of each workspace off screen since
//! [`crate::NavigatorConfig::stop_after`] or longer, both sides, and forgets their keys:
//! they are not listed in [`crate::Told::Servers`] any more. A workspace that comes back
//! on screen has its new side's servers ensured again; an old side's server starts again
//! at the next ask about the old side. A workspace that leaves the list of [`crate::Ask::Workspaces`] has every
//! server stopped at once. [`close`] stops every server, each with
//! [`crate::STOP_DEADLINE`], and returns when every process is gone.
//!
//! # What other modules call
//!
//! `answers.rs` and `old_side.rs` call [`ensure`], [`ready_server`], [`state_of`],
//! [`stop`] and [`publish`]; nothing else here.

use std::path::Path;

use yaar_link::WorkspaceId;

use crate::download::{DownloadError, Installed};
use crate::driver::{Launch, Server, ServerEvent};
use crate::navigator::{Core, ServerKey};
use crate::port::{RepositoryConsent, State, WorkspaceRoot};
use crate::search::Candidate;

/// What this module keeps in [`Core::life`]: every key ensured with where it stands, the
/// servers that run, the times workspaces left the screen.
#[derive(Default)]
pub(crate) struct Life {}

/// At the start of the Navigator: reads the registry from
/// [`crate::NavigatorConfig::builtin`] and the user's file into [`Core::registry`],
/// writing each [`crate::registry::RowError`] to the standard error as one line; tells
/// [`crate::Told::Languages`], each row's name, title and extensions in the registry's
/// order; and removes every folder `<cache>/<pid>` whose process no longer runs.
pub(crate) fn started(core: &mut Core) {
    let _ = core;
    todo!()
}

/// [`crate::Ask::Workspaces`]: keeps the list; every key of a workspace no longer listed, or
/// whose worktree is gone, is stopped and forgotten.
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

/// [`crate::Ask::Download`]: downloads the row's server for every key whose state offers it.
pub(crate) fn download(core: &mut Core, language: &str) {
    let _ = (core, language);
    todo!()
}

/// [`crate::Ask::Update`]: downloads the version the row pins now, for a row whose state shows an
/// update, and starts it in the place of the one that runs.
pub(crate) fn update(core: &mut Core, language: &str) {
    let _ = (core, language);
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

/// A download ended: the version installed is started, or the offer shown again.
pub(crate) fn downloaded(
    core: &mut Core,
    language: String,
    outcome: Result<Installed, DownloadError>,
) {
    let _ = (core, language, outcome);
    todo!()
}

/// Stops the servers of every workspace off screen for the set time or longer, as the
/// module doc says.
pub(crate) fn look_at_the_clock(core: &mut Core) {
    let _ = core;
    todo!()
}

/// At the end: stops every server and waits until each is gone.
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

/// Stops the server of `key`, if one runs, and forgets the key.
pub(crate) fn stop(core: &mut Core, key: &ServerKey) {
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
