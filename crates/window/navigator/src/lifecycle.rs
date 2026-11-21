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
//! after each change of any key ([`publish`]): one list for each event handled, which
//! [`on_screen`] always tells, and before it, when it stopped any, the list of
//! [`look_at_the_clock`]. A test that sends an ask that changes nothing reads the list that
//! ask made, after everything the asks before it made the Navigator do:
//!
//! 1. Search: `search::look` and `search::order` give the candidates, kept as the list of
//!    the key's candidates left. Each candidate that lies inside the workspace's repository
//!    (`search::inside`, against the folders of every workspace of that repository, and the
//!    export's folder for an old side) is
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
//!    `{target}` `<cache>/navigator/targets/<workspace>/new` or `old`, `{platform}`, `{servers}` the
//!    downloaded version's folder of that server. Ready at [`ServerEvent::Ready`]. A start
//!    that fails, an exit before the server was ready, and an exit after it, are each a
//!    failure of that candidate: the next one is tried (FR-061). `answers::server_gone`
//!    hears of the exit of a server that had been ready, and of its end when this module
//!    stops it itself ([`stop`], [`search_again`], a download that takes its key's place):
//!    a ready server is the only kind a request was ever sent to, and one that ended before
//!    it was ready has no ask to answer.
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
//! | ready | [`State::Ready`], with `update` when the program is a downloaded one of another version than the row pins | [`ServerEvent::Exited`] | the next candidate left, or none left |
//! | none left | `search::when_none` | [`search_again`] (a yes, or `fetch.rs`) | a new search |
//! | downloading | [`State::Downloading`] | [`search_again`], from `fetch.rs` | a new search |
//! | waiting for the shell | [`State::Starting`] | [`shell_taken`] | a new search, with the environment it gave |
//!
//! "Waiting for the shell" (slice 9; the module `shell.rs`): with
//! `NavigatorConfig::login_shell` some and the shell not yet taken, the first key that
//! would be searched starts the shell's thread instead (`shell::take` on a thread of its
//! own, reporting `Event::Shell`), and it and every key that would be searched after it,
//! before the report, are held in this phase, with no generation counted. [`shell_taken`]
//! puts the environment the shell gave (or, when it failed, leaves the configuration's, and
//! writes the line `shell.rs` names to the standard error) into
//! `NavigatorConfig::environment` and `NavigatorConfig::search_path`, notes the shell as
//! taken, and searches every held key, in the order they were held, as a first search does.
//! A held key stopped meanwhile is forgotten and not searched. With `login_shell` none, or
//! the shell taken, no key is ever held.
//!
//! "The next candidate left": the head of the key's list is taken off it; one inside the
//! repository is skipped as step 1 says; one whose server names `trial` is tried, any other
//! started. An empty list is "none left". Any phase goes when the key is stopped
//! ([`stop`]): its server stopped, or its trial run killed, the key forgotten.
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
//! `<cache>/navigator/targets/<workspace>`, removed. [`close`] drops the asks still waiting and stops
//! every server.
//!
//! Every stop here is done before the function that does it returns: [`stop`] calls
//! `Server::stop` with [`crate::STOP_DEADLINE`], which returns once the process is reaped
//! (a trial run on its way is killed and reaped the same way, so that none outlives the key
//! or the Navigator), and only then forgets the key and publishes. So
//! [`look_at_the_clock`], [`workspaces`], [`close`], and `old_side.rs` through [`stop`], have
//! every process they stop gone when they return, and a [`crate::Told::Servers`] told after
//! them names no process that still runs. Nothing here waits for a server's answer or its
//! readiness: those come as events.
//!
//! # What other modules call
//!
//! `answers.rs` and `old_side.rs` call [`ensure`], [`ready_server`], [`running_server`],
//! [`state_of`], [`stop`] and [`publish`]; `fetch.rs` calls [`offering`], [`updating`],
//! [`on_an_older_copy`], [`downloading`], [`search_again`] and [`publish`]; nothing else here.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use yaar_link::{RepositoryId, WorkspaceId};

use crate::download::{Installed, installed};
use crate::driver::{Launch, Server, ServerEvent};
use crate::navigator::{Core, Event, ServerKey, Side};
use crate::port::{Language, Offer, RepositoryConsent, ServerState, State, Told, WorkspaceRoot};
use crate::registry::{Registry, Row, Source, Vars, expand, expand_json};
use crate::search::{Candidate, CandidateKind, inside, look, order, when_none};
use crate::{STOP_DEADLINE, TRIAL_DEADLINE, answers};

/// What this module keeps in [`Core::life`]: every key ensured with where it stands, the
/// servers that run, the times workspaces left the screen.
#[derive(Default)]
pub(crate) struct Life {
    /// Every key known.
    keys: BTreeMap<ServerKey, Keyed>,
    /// When each workspace that is off screen left it, or was first looked at while it was.
    left: HashMap<WorkspaceId, Instant>,
    /// How many servers this Navigator started: the number of the next one's log is one
    /// more.
    started: u64,
    /// The last generation given out. One count for every key, so that an event of a key
    /// that was stopped can never be taken for one of the key made again in its place.
    generation: u64,
}

/// One key and where it stands.
struct Keyed {
    /// The folder it serves: the worktree's, or the export's.
    root: PathBuf,
    /// Which search the key is in. An event of another is dropped.
    generation: u64,
    /// The candidates not tried yet, in the order they are.
    left: VecDeque<Candidate>,
    /// The phase of the table in the module doc.
    phase: Phase,
    /// The question the key keeps: the repository and the first candidate inside it that was
    /// passed over for want of an answer. Kept beside the phase: the key carries on with
    /// the candidates after that one.
    question: Option<(RepositoryId, PathBuf)>,
    /// Whether this search passed over a candidate for the repository's answer, asked or
    /// refused: a yes searches the key again, for that candidate comes before the one that
    /// runs.
    passed_over: bool,
}

/// A server of a key that runs, and what it is a copy of.
struct Running {
    /// The candidate it was started from.
    candidate: Candidate,
    /// The process.
    server: Server,
    /// For a downloaded copy, its version.
    version: Option<String>,
}

/// The phases of a key.
enum Phase {
    /// A trial run of the candidate is on its way. Its process is shared with the thread
    /// that waits for it, so that stopping the key can end it too.
    Trying {
        candidate: Candidate,
        process: Arc<Mutex<Option<Child>>>,
    },
    /// The server runs and its rule of readiness is not met.
    Starting { running: Running },
    /// The server runs and is ready.
    Ready { running: Running },
    /// No candidate is left.
    NoneLeft,
    /// The download of a server for the row is on its way.
    Downloading { offer: Offer },
}

/// At the start of the Navigator: reads the registry from
/// [`crate::NavigatorConfig::builtin`] and the user's file into [`Core::registry`],
/// writing each [`crate::registry::RowError`] to the standard error as one line; tells
/// [`crate::Told::Languages`], each row's name, title and extensions in the registry's
/// order; and removes every folder `<cache>/navigator/<pid>-<k>` whose `<pid>`, the part of its name
/// before the `-`, names no process that runs (a folder whose name is not of that form is
/// left as it is).
pub(crate) fn started(core: &mut Core) {
    let user = match std::fs::read_to_string(&core.config.user_file) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            eprintln!(
                "language servers: cannot read {}: {error}",
                core.config.user_file.display()
            );
            None
        }
    };
    let (registry, errors) = Registry::read(&core.config.builtin, user.as_deref());
    for error in errors {
        let place = match error.source {
            Source::Builtin => "the built-in table",
            Source::User => "the user's file",
        };
        match error.row {
            Some(row) => eprintln!(
                "language servers: the row `{row}` of {place} is passed over: {}",
                error.reason
            ),
            None => eprintln!("language servers: {place} {}", error.reason),
        }
    }
    core.registry = registry;
    let languages = core
        .registry
        .rows()
        .iter()
        .map(|row| Language {
            name: row.name.clone(),
            title: row.title.clone(),
            extensions: row.extensions.iter().map(|(e, _)| e.clone()).collect(),
        })
        .collect();
    core.tell(Told::Languages(languages));
    remove_stale_caches(&core.config.cache);
}

/// Removes each folder of `cache` named `<pid>-<k>` (two numbers) whose process does not
/// run: the own cache of a Window that ended without removing it.
fn remove_stale_caches(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let number = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some((pid, k)) = name.to_str().and_then(|name| name.split_once('-')) else {
            continue;
        };
        if !number(pid) || !number(k) {
            continue;
        }
        // A number too large for a process id, and 0, which names a group and not a process,
        // are no process of ours either way: left alone.
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid == 0 || process_runs(pid) {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether a process of this id exists, whoever's it is.
fn process_runs(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only asks whether the process exists; it sends nothing.
    let signalled = unsafe { libc::kill(pid, 0) };
    // A process of another user exists, and answers that it may not be signalled.
    signalled == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// [`crate::Ask::Workspaces`]: keeps the list; every key of a workspace no longer listed, or
/// whose worktree is gone, is stopped and forgotten; a workspace listed before and no
/// longer listed has `<cache>/navigator/targets/<workspace>` removed.
pub(crate) fn workspaces(core: &mut Core, roots: Vec<WorkspaceRoot>) {
    let before: Vec<WorkspaceId> = core.workspaces.iter().map(|root| root.id.clone()).collect();
    core.workspaces = roots;

    let gone: Vec<ServerKey> = core
        .life
        .keys
        .iter()
        .filter(|(key, keyed)| {
            match core.workspaces.iter().find(|root| root.id == key.workspace) {
                None => true,
                Some(root) => match &root.folder {
                    None => true,
                    // The new side serves the worktree's folder: a workspace whose folder
                    // moved is another place. An old side serves an export, which
                    // `old_side.rs` keeps.
                    Some(folder) => key.side == Side::New && *folder != keyed.root,
                },
            }
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in &gone {
        stop_quietly(core, key);
    }

    for id in before {
        if !core.workspaces.iter().any(|root| root.id == id) {
            core.life.left.remove(&id);
            let _ = std::fs::remove_dir_all(core.config.cache.join("targets").join(id.as_str()));
        }
    }

    // A workspace on screen that the list now holds, or holds somewhere else, has its
    // servers.
    if let Some(on_screen) = core.on_screen.clone() {
        ensure_by_markers(core, &on_screen);
    }
    publish(core);
}

/// [`crate::Ask::OnScreen`]: the workspace that left the screen has the time noted; the one that
/// came on screen has the time taken away and its new side's servers ensured, by markers.
/// It ends with [`publish`] every time, also when nothing changed: what it tells is what a
/// test that moved its clock reads after the clock was looked at.
pub(crate) fn on_screen(core: &mut Core, workspace: Option<WorkspaceId>) {
    let now = core.config.clock.now();
    let previous = core.on_screen.take();
    if let Some(previous) = previous
        && Some(&previous) != workspace.as_ref()
    {
        core.life.left.insert(previous, now);
    }
    core.on_screen = workspace.clone();
    if let Some(workspace) = workspace {
        core.life.left.remove(&workspace);
        ensure_by_markers(core, &workspace);
    }
    publish(core);
}

/// Ensures the new side's server of each row whose `markers` lie at the root of the
/// worktree of `workspace`. Tells nothing.
fn ensure_by_markers(core: &mut Core, workspace: &WorkspaceId) {
    let Some(folder) = core.folder_of(workspace) else {
        return;
    };
    let languages: Vec<String> = core
        .registry
        .rows()
        .iter()
        .filter(|row| {
            row.markers
                .iter()
                .any(|marker| folder.join(marker).exists())
        })
        .map(|row| row.name.clone())
        .collect();
    for language in languages {
        let key = ServerKey {
            workspace: workspace.clone(),
            language,
            side: Side::New,
        };
        ensure_key(core, &key, &folder);
    }
}

/// [`crate::Ask::Consents`]: keeps the answers, and searches again as the module doc says.
pub(crate) fn consents(core: &mut Core, consents: Vec<RepositoryConsent>) {
    let before = std::mem::replace(&mut core.consents, consents);
    let answer = |list: &[RepositoryConsent], repository: &RepositoryId| {
        list.iter()
            .find(|consent| &consent.repository == repository)
            .map(|consent| consent.allowed)
    };
    let repositories: BTreeSet<RepositoryId> = before
        .iter()
        .chain(&core.consents)
        .map(|consent| consent.repository.clone())
        .collect();

    let mut changed = false;
    for repository in repositories {
        let (was, now) = (
            answer(&before, &repository),
            answer(&core.consents, &repository),
        );
        if was == now {
            continue;
        }
        let keys: Vec<ServerKey> = core
            .life
            .keys
            .keys()
            .filter(|key| repository_of(core, &key.workspace) == Some(&repository))
            .cloned()
            .collect();
        for key in keys {
            match now {
                // The candidate passed over comes before the one that runs, or the key has
                // none: from the start again.
                Some(true) if core.life.keys[&key].passed_over => {
                    restart(core, &key);
                    changed = true;
                }
                // The candidates stay passed over, and no question is left to wait for.
                Some(false) => {
                    if let Some(keyed) = core.life.keys.get_mut(&key)
                        && keyed.question.take().is_some()
                    {
                        changed = true;
                    }
                }
                _ => {}
            }
        }
    }
    if changed {
        publish(core);
    }
}

/// What a server's reading thread told: ready, an answer (handed to
/// `answers::server_answered`), or its exit. Only the exit of a server that had been ready
/// is handed to `answers::server_gone`, since only such a server was ever sent a request;
/// after any exit of the server a key was running, the next candidate is tried. An event of
/// an earlier generation of the key is dropped.
pub(crate) fn server_event(core: &mut Core, key: ServerKey, generation: u64, event: ServerEvent) {
    let Some(keyed) = core.life.keys.get_mut(&key) else {
        return;
    };
    if keyed.generation != generation {
        return;
    }
    match event {
        ServerEvent::Ready => {
            let phase = std::mem::replace(&mut keyed.phase, Phase::NoneLeft);
            keyed.phase = match phase {
                Phase::Starting { running } => Phase::Ready { running },
                other => other,
            };
            publish(core);
        }
        ServerEvent::Answered { request, result } => {
            answers::server_answered(core, &key, request, result);
        }
        ServerEvent::Exited { .. } => {
            let phase = std::mem::replace(&mut keyed.phase, Phase::NoneLeft);
            let was_ready = match phase {
                Phase::Ready { .. } => true,
                Phase::Starting { .. } => false,
                // No server of this search is running: the exit is of one already replaced.
                other => {
                    keyed.phase = other;
                    return;
                }
            };
            // Only a server that was ready was ever sent a request, so only its exit can
            // leave an ask waiting. The key stands as "none left" meanwhile.
            if was_ready {
                answers::server_gone(core, &key);
            }
            let Some(mut keyed) = core.life.keys.remove(&key) else {
                return;
            };
            advance(core, &key, &mut keyed);
            core.life.keys.insert(key, keyed);
            publish(core);
        }
    }
}

/// A trial run ended: the candidate is started, or passed over for the next.
pub(crate) fn trial_ended(
    core: &mut Core,
    key: ServerKey,
    generation: u64,
    candidate: Candidate,
    passed: bool,
) {
    let Some(mut keyed) = core.life.keys.remove(&key) else {
        return;
    };
    let current = keyed.generation == generation
        && matches!(&keyed.phase, Phase::Trying { candidate: tried, .. } if *tried == candidate);
    if current {
        if passed {
            match start(core, &key, &keyed, candidate) {
                Ok(phase) => keyed.phase = phase,
                Err(reason) => {
                    eprintln!("language servers: {reason}");
                    advance(core, &key, &mut keyed);
                }
            }
        } else {
            advance(core, &key, &mut keyed);
        }
    }
    core.life.keys.insert(key, keyed);
    if current {
        publish(core);
    }
}

/// Stops the servers of every workspace off screen for the set time or longer, as the
/// module doc says.
pub(crate) fn look_at_the_clock(core: &mut Core) {
    if core.life.left.is_empty() {
        return;
    }
    let now = core.config.clock.now();
    let due: Vec<WorkspaceId> = core
        .life
        .left
        .iter()
        .filter(|(_, since)| now.saturating_duration_since(**since) >= core.config.stop_after)
        .map(|(workspace, _)| workspace.clone())
        .collect();
    let mut stopped = false;
    for workspace in due {
        core.life.left.remove(&workspace);
        let keys: Vec<ServerKey> = core
            .life
            .keys
            .keys()
            .filter(|key| key.workspace == workspace)
            .cloned()
            .collect();
        for key in keys {
            stopped |= stop_quietly(core, &key);
        }
    }
    if stopped {
        publish(core);
    }
}

/// The user's login shell gave its environment, or failed to, as `shell.rs` says: takes it
/// and searches the keys held for it, as the table "Where a key stands" says.
pub(crate) fn shell_taken(
    core: &mut Core,
    taken: Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, crate::shell::ShellError>,
) {
    let _ = (core, taken);
    todo!("take the environment and search the held keys, as the module doc says")
}

/// At the end: drops the asks still waiting, then stops every server, each as [`stop`] does.
pub(crate) fn close(core: &mut Core) {
    // The asks waiting at these servers are dropped first: the end of a server answers them
    // by text match, and no one is left to read it.
    answers::forget_the_waiting(core);
    let keys: Vec<ServerKey> = core.life.keys.keys().cloned().collect();
    for key in &keys {
        stop_quietly(core, key);
    }
    publish(core);
}

/// Makes sure the server of `key` is running or being looked for, `root` the folder it
/// serves: the worktree's folder for [`crate::navigator::Side::New`], the export's for an
/// old side. Starts the search when the key is not known yet. Gives where it stands now.
pub(crate) fn ensure(core: &mut Core, key: &ServerKey, root: &Path) -> State {
    if ensure_key(core, key, root) {
        publish(core);
    }
    state_of(core, key).unwrap_or(State::Missing { offer: None })
}

/// [`ensure`] without telling. Gives whether the key was made.
fn ensure_key(core: &mut Core, key: &ServerKey, root: &Path) -> bool {
    if core.life.keys.contains_key(key) {
        return false;
    }
    // A workspace that is not on screen has its servers stopped after the set time, from
    // the first one asked for.
    if core.on_screen.as_ref() != Some(&key.workspace)
        && !core.life.left.contains_key(&key.workspace)
    {
        let now = core.config.clock.now();
        core.life.left.insert(key.workspace.clone(), now);
    }
    core.life.generation += 1;
    let mut keyed = Keyed {
        root: root.to_path_buf(),
        generation: core.life.generation,
        left: VecDeque::new(),
        phase: Phase::NoneLeft,
        question: None,
        passed_over: false,
    };
    search(core, key, &mut keyed);
    advance(core, key, &mut keyed);
    core.life.keys.insert(key.clone(), keyed);
    true
}

/// The server of `key`, when one runs and is ready.
pub(crate) fn ready_server<'a>(core: &'a Core, key: &ServerKey) -> Option<&'a Server> {
    match &core.life.keys.get(key)?.phase {
        Phase::Ready { running } => Some(&running.server),
        _ => None,
    }
}

/// The server of `key`, when one runs, starting or ready: the process that can be told what
/// changed. One whose rule of readiness is not met yet reads the files when it loads, and the
/// driver holds what is sent before the answer to `initialize`.
pub(crate) fn running_server<'a>(core: &'a Core, key: &ServerKey) -> Option<&'a Server> {
    match &core.life.keys.get(key)?.phase {
        Phase::Starting { running } | Phase::Ready { running } => Some(&running.server),
        _ => None,
    }
}

/// Where the server of `key` stands; none for a key not known.
pub(crate) fn state_of(core: &Core, key: &ServerKey) -> Option<State> {
    core.life
        .keys
        .get(key)
        .map(|keyed| shown(core, key, keyed).state)
}

/// Stops the server of `key`, if one runs, with `Server::stop` and
/// [`crate::STOP_DEADLINE`], which returns once its process is reaped; then forgets the key
/// and publishes.
pub(crate) fn stop(core: &mut Core, key: &ServerKey) {
    stop_quietly(core, key);
    publish(core);
}

/// [`stop`] without telling. Gives whether the key was known.
fn stop_quietly(core: &mut Core, key: &ServerKey) -> bool {
    let Some(mut keyed) = core.life.keys.remove(key) else {
        return false;
    };
    if end_server(&mut keyed) {
        answers::server_gone(core, key);
    }
    true
}

/// Stops the server of the key's phase, or kills the trial run that is on its way, if
/// there is one, and waits for its process to be reaped; the phase is none left after.
/// Gives whether the server that ended was a ready one, the only kind a request was ever
/// sent to: the caller then hands the key to `answers::server_gone`, once the key stands
/// as it will after the stop.
fn end_server(keyed: &mut Keyed) -> bool {
    match std::mem::replace(&mut keyed.phase, Phase::NoneLeft) {
        Phase::Starting { running } => running.server.stop(STOP_DEADLINE),
        Phase::Ready { running } => {
            running.server.stop(STOP_DEADLINE);
            return true;
        }
        Phase::Trying { process, .. } => {
            // Taken out first, so that the thread that waits for it finds it gone and the
            // lock is not held while the process is reaped.
            let taken = process
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            kill_and_reap(taken);
        }
        Phase::NoneLeft | Phase::Downloading { .. } => {}
    }
    false
}

/// The keys of the row `language` whose state is missing with an offer, each with it.
pub(crate) fn offering(core: &Core, language: &str) -> Vec<(ServerKey, Offer)> {
    core.life
        .keys
        .iter()
        .filter(|(key, _)| key.language == language)
        .filter_map(|(key, keyed)| match shown(core, key, keyed).state {
            State::Missing { offer: Some(offer) } => Some((key.clone(), offer)),
            _ => None,
        })
        .collect()
}

/// The keys of the row `language` whose state shows an update, each with it.
pub(crate) fn updating(core: &Core, language: &str) -> Vec<(ServerKey, Offer)> {
    core.life
        .keys
        .iter()
        .filter(|(key, _)| key.language == language)
        .filter_map(|(key, keyed)| update_of(core, key, keyed).map(|offer| (key.clone(), offer)))
        .collect()
}

/// The keys of the row `language` whose candidate, the one tried, started or running, is a
/// downloaded copy of the server named `server` in a version other than `installed`, the one
/// just installed: the folder of the copy they were given is gone or about to go, and a key
/// that does not search again finds it missing, or runs from a folder that is not there. The
/// phases trying, starting and ready all count: a candidate is chosen by the version that was
/// installed when the key searched, whatever the key has reached since.
pub(crate) fn on_an_older_copy(
    core: &Core,
    language: &str,
    server: &str,
    installed: &Installed,
) -> Vec<ServerKey> {
    let Some(row) = core.registry.row(language) else {
        return Vec::new();
    };
    let current = core.config.servers.join(server).join(&installed.version);
    core.life
        .keys
        .iter()
        .filter(|(key, _)| key.language == language)
        .filter(|(_, keyed)| {
            let candidate = match &keyed.phase {
                Phase::Trying { candidate, .. } => candidate,
                Phase::Starting { running } | Phase::Ready { running } => &running.candidate,
                Phase::NoneLeft | Phase::Downloading { .. } => return false,
            };
            candidate.kind == CandidateKind::Downloaded
                && row
                    .servers
                    .get(candidate.server)
                    .is_some_and(|candidate_server| candidate_server.name == server)
                && !candidate.program.starts_with(&current)
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// Shows `key` as [`State::Downloading`] with `offer` until it is searched again, and
/// publishes.
pub(crate) fn downloading(core: &mut Core, key: &ServerKey, offer: Offer) {
    let Some(keyed) = core.life.keys.get_mut(key) else {
        return;
    };
    let was_ready = end_server(keyed);
    keyed.phase = Phase::Downloading { offer };
    // `fetch::download` hands over only keys that offer a download, which run nothing, so
    // no ask waits at one today; the call keeps a caller that hands over a ready key from
    // leaving its asks waiting.
    if was_ready {
        answers::server_gone(core, key);
    }
    publish(core);
}

/// Searches again for `key`, from step 1 of the module doc, with the root it was ensured
/// with: stops its server if one runs (as [`stop`] does, without forgetting the key),
/// raises its generation, and publishes. A key not known is left alone.
pub(crate) fn search_again(core: &mut Core, key: &ServerKey) {
    if core.life.keys.contains_key(key) {
        restart(core, key);
        publish(core);
    }
}

/// [`search_again`] without telling.
fn restart(core: &mut Core, key: &ServerKey) {
    let Some(mut keyed) = core.life.keys.remove(key) else {
        return;
    };
    if end_server(&mut keyed) {
        answers::server_gone(core, key);
    }
    core.life.generation += 1;
    keyed.generation = core.life.generation;
    search(core, key, &mut keyed);
    advance(core, key, &mut keyed);
    core.life.keys.insert(key.clone(), keyed);
}

/// Tells [`crate::Told::Servers`]: every key known, in the order the port's doc says.
pub(crate) fn publish(core: &Core) {
    let place = |key: &ServerKey| {
        (
            core.workspaces
                .iter()
                .position(|root| root.id == key.workspace)
                .unwrap_or(usize::MAX),
            core.registry
                .rows()
                .iter()
                .position(|row| row.name == key.language)
                .unwrap_or(usize::MAX),
            key.side.clone(),
        )
    };
    let mut keys: Vec<(&ServerKey, &Keyed)> = core.life.keys.iter().collect();
    keys.sort_by_key(|(key, _)| place(key));
    let states = keys
        .into_iter()
        .map(|(key, keyed)| shown(core, key, keyed))
        .collect();
    core.tell(Told::Servers(states));
}

/// The state of a key as the port tells it.
fn shown(core: &Core, key: &ServerKey, keyed: &Keyed) -> ServerState {
    let row = core.registry.row(&key.language);
    let name = |candidate: &Candidate| {
        row.and_then(|row| row.servers.get(candidate.server))
            .map(|server| server.name.clone())
    };
    let (server, program, pid) = match &keyed.phase {
        Phase::Trying { candidate, .. } => (name(candidate), None, None),
        Phase::Starting { running } | Phase::Ready { running } => (
            name(&running.candidate),
            Some(running.candidate.program.clone()),
            Some(running.server.pid()),
        ),
        Phase::NoneLeft | Phase::Downloading { .. } => (None, None, None),
    };
    // The question takes the line while the key keeps one, whatever the phase.
    let state = if let Some((repository, program)) = &keyed.question {
        State::Asking {
            repository: repository.clone(),
            program: program.clone(),
        }
    } else {
        match &keyed.phase {
            Phase::Trying { .. } | Phase::Starting { .. } => State::Starting,
            Phase::Ready { .. } => State::Ready,
            Phase::NoneLeft => row.map_or(State::Missing { offer: None }, |row| {
                when_none(row, core.config.platform)
            }),
            Phase::Downloading { offer } => State::Downloading {
                offer: offer.clone(),
            },
        }
    };
    ServerState {
        workspace: key.workspace.clone(),
        language: key.language.clone(),
        title: row.map_or_else(|| key.language.clone(), |row| row.title.clone()),
        old_side: match &key.side {
            Side::New => None,
            Side::Old { version } => Some(version.clone()),
        },
        registered: row.is_some_and(|row| row.registered),
        state,
        server,
        program,
        pid,
        update: update_of(core, key, keyed),
    }
}

/// The update a ready server shows: the version the row pins for this machine, when the
/// running copy is a downloaded one of another version.
fn update_of(core: &Core, key: &ServerKey, keyed: &Keyed) -> Option<Offer> {
    let Phase::Ready { running } = &keyed.phase else {
        return None;
    };
    let version = running.version.as_ref()?;
    let server = core
        .registry
        .row(&key.language)?
        .servers
        .get(running.candidate.server)?;
    let pinned = server.download(core.config.platform?)?;
    (pinned.version != *version).then(|| Offer {
        server: server.name.clone(),
        version: pinned.version.clone(),
        size: pinned.size,
    })
}

/// The repository of a workspace the Navigator was told of.
fn repository_of<'a>(core: &'a Core, workspace: &WorkspaceId) -> Option<&'a RepositoryId> {
    core.workspaces
        .iter()
        .find(|root| &root.id == workspace)
        .map(|root| &root.repository)
}

/// Looks for the candidates of `key`, from the start: the list left is every candidate in
/// the order of FR-060, and no question or passed over candidate is kept.
fn search(core: &Core, key: &ServerKey, keyed: &mut Keyed) {
    keyed.left.clear();
    keyed.question = None;
    keyed.passed_over = false;
    let Some(row) = core.registry.row(&key.language) else {
        return;
    };
    let found = look(
        row,
        &keyed.root,
        &core.config.search_path,
        &core.config.environment,
        &core.config.servers,
        core.config.platform,
    );
    keyed.left = order(row, &found).into();
}

/// Takes candidates off the key's list until one is started or being tried, as step 1 to 3
/// of the module doc say; with none left the phase is none left.
fn advance(core: &mut Core, key: &ServerKey, keyed: &mut Keyed) {
    loop {
        let Some(candidate) = keyed.left.pop_front() else {
            keyed.phase = Phase::NoneLeft;
            return;
        };
        if let Some(repository) = repository_of(core, &key.workspace).cloned()
            && lies_inside(core, key, keyed, &repository, &candidate)
        {
            let answer = core
                .consents
                .iter()
                .find(|consent| consent.repository == repository)
                .map(|consent| consent.allowed);
            match answer {
                Some(true) => {}
                Some(false) => {
                    keyed.passed_over = true;
                    continue;
                }
                None => {
                    keyed.passed_over = true;
                    if keyed.question.is_none() {
                        keyed.question = Some((repository, candidate.program.clone()));
                    }
                    continue;
                }
            }
        }
        let has_trial = core
            .registry
            .row(&key.language)
            .and_then(|row| row.servers.get(candidate.server))
            .is_some_and(|server| server.trial.is_some());
        let begun = if has_trial {
            try_out(core, key, keyed, candidate)
        } else {
            start(core, key, keyed, candidate)
        };
        match begun {
            Ok(phase) => {
                keyed.phase = phase;
                return;
            }
            Err(reason) => eprintln!("language servers: {reason}"),
        }
    }
}

/// Whether the candidate's program lies inside a worktree of `repository`: the folder of
/// each workspace of it the Navigator was told of, and for an old side the export too, which
/// holds a copy of what the repository holds.
fn lies_inside(
    core: &Core,
    key: &ServerKey,
    keyed: &Keyed,
    repository: &RepositoryId,
    candidate: &Candidate,
) -> bool {
    let mut folders: Vec<&Path> = core
        .workspaces
        .iter()
        .filter(|root| &root.repository == repository)
        .filter_map(|root| root.folder.as_deref())
        .collect();
    if matches!(key.side, Side::Old { .. }) {
        folders.push(&keyed.root);
    }
    inside(&candidate.program, &folders)
}

/// The row and server a candidate is a copy of.
fn server_of<'a>(
    core: &'a Core,
    key: &ServerKey,
    candidate: &Candidate,
) -> Result<(&'a Row, &'a crate::registry::Server), String> {
    let row = core
        .registry
        .row(&key.language)
        .ok_or_else(|| format!("no row is named {}", key.language))?;
    let server = row.servers.get(candidate.server).ok_or_else(|| {
        format!(
            "the row {} has no server {} of the candidate",
            key.language, candidate.server
        )
    })?;
    Ok((row, server))
}

/// Starts the trial run of `candidate`, and a thread of its own that waits for it and tells
/// [`Event::Trial`]. The process belongs to the phase too, so that stopping the key ends it.
fn try_out(
    core: &Core,
    key: &ServerKey,
    keyed: &Keyed,
    candidate: Candidate,
) -> Result<Phase, String> {
    let launch = launch(core, key, &keyed.root, &candidate)?;
    let (_, server) = server_of(core, key, &candidate)?;
    let expander = expander(core, key, &keyed.root, &candidate);
    let args = server
        .trial
        .iter()
        .flatten()
        .map(|arg| expander.text(arg))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|reason| trial_fault(&candidate, &reason))?;
    let child =
        spawn_trial(&launch, &args).map_err(|error| trial_fault(&candidate, &error.to_string()))?;
    let process = Arc::new(Mutex::new(Some(child)));

    let events = core.events.clone();
    let (key, generation, tried) = (key.clone(), keyed.generation, candidate.clone());
    let waited_for = Arc::clone(&process);
    let thread = std::thread::Builder::new()
        .name("navigator-trial".to_owned())
        .spawn(move || {
            let passed = wait_for_trial(&waited_for);
            let _ = events.send(Event::Trial {
                key,
                generation,
                candidate: tried,
                passed,
            });
        });
    if let Err(error) = thread {
        let taken = process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        kill_and_reap(taken);
        return Err(format!("cannot start a thread for a trial run: {error}"));
    }
    Ok(Phase::Trying { candidate, process })
}

fn trial_fault(candidate: &Candidate, reason: &str) -> String {
    format!("{} cannot be tried: {reason}", candidate.program.display())
}

/// Starts the program of `launch` once with `args`, in its environment and its folder.
fn spawn_trial(launch: &Launch, args: &[String]) -> std::io::Result<Child> {
    let mut command = Command::new(&launch.program);
    command
        .args(args)
        .current_dir(&launch.root)
        .env_clear()
        .envs(
            launch
                .environment
                .iter()
                .map(|(name, value)| (name.as_os_str(), value.as_os_str())),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    end_with_the_window(&mut command);
    command.spawn()
}

/// Asks the kernel to kill the trial run when the thread that starts it ends, on Linux, as
/// the driver does for a server. On macOS nothing ends a run whose Window crashed, and a
/// trial is short.
#[cfg(target_os = "linux")]
fn end_with_the_window(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: `getpid` has no preconditions.
    let parent = unsafe { libc::getpid() };
    // SAFETY: between the fork and the exec the closure calls `prctl`, `getppid` and `_exit`
    // only, which are safe there, and touches no memory of this process's.
    unsafe {
        command.pre_exec(move || {
            // The closure is part of the unsafe block above, whose comment covers these
            // calls.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have ended before the request took effect: its signal
            // would never come.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn end_with_the_window(_command: &mut Command) {}

/// Kills the trial run, if there is one, and waits until it is reaped.
fn kill_and_reap(child: Option<Child>) {
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Waits for the trial run in `process`, and says whether it ended with success within
/// [`TRIAL_DEADLINE`]; a run that takes longer is killed. A run the key's stop took away is
/// no success.
fn wait_for_trial(process: &Mutex<Option<Child>>) -> bool {
    let until = Instant::now() + TRIAL_DEADLINE;
    // A repeated look at one fact that stays true once it is: the process ended. A trial is
    // short, so the first looks come quickly. The lock is held for a look only.
    let mut pause = Duration::from_millis(1);
    loop {
        {
            let mut held = process.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(child) = held.as_mut() else {
                return false;
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    held.take();
                    return status.success();
                }
                Ok(None) if Instant::now() < until => {}
                _ => {
                    kill_and_reap(held.take());
                    return false;
                }
            }
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(20));
    }
}

/// Starts the server of `candidate`: the phase is starting.
fn start(
    core: &mut Core,
    key: &ServerKey,
    keyed: &Keyed,
    candidate: Candidate,
) -> Result<Phase, String> {
    let launch = launch(core, key, &keyed.root, &candidate)?;
    let (_, server) = server_of(core, key, &candidate)?;
    let version = (candidate.kind == CandidateKind::Downloaded)
        .then(|| installed(&core.config.servers, &server.name).map(|installed| installed.version))
        .flatten();
    core.life.started += 1;

    let events = core.events.clone();
    let (named, generation) = (key.clone(), keyed.generation);
    let program = launch.program.clone();
    let server = Server::start(
        launch,
        Box::new(move |event| {
            let _ = events.send(Event::Server {
                key: named.clone(),
                generation,
                event,
            });
        }),
    )
    .map_err(|reason| format!("{}: {reason}", program.display()))?;
    Ok(Phase::Starting {
        running: Running {
            candidate,
            server,
            version,
        },
    })
}

/// Puts in the variables of a row for one candidate of one key.
struct Expander {
    vars: Vars,
    /// Whether `{servers}` has a value: only a downloaded copy has a folder.
    servers: bool,
}

impl Expander {
    /// `text` with its variables put in; an error when it names `{servers}` and the copy has
    /// no such folder.
    fn text(&self, text: &str) -> Result<String, String> {
        let expanded = expand(text, &self.vars)?;
        if !self.servers {
            let probe = Vars {
                servers: "other".to_owned(),
                ..self.vars.clone()
            };
            if expand(text, &probe)? != expanded {
                return Err(NO_SERVERS_FOLDER.to_owned());
            }
        }
        Ok(expanded)
    }

    /// [`Expander::text`] for every string of `value`.
    fn json(&self, value: &serde_json::Value) -> Result<serde_json::Value, String> {
        let expanded = expand_json(value, &self.vars)?;
        if !self.servers {
            let probe = Vars {
                servers: "other".to_owned(),
                ..self.vars.clone()
            };
            if expand_json(value, &probe)? != expanded {
                return Err(NO_SERVERS_FOLDER.to_owned());
            }
        }
        Ok(expanded)
    }
}

/// Why a row that names `{servers}` cannot start a copy that was not downloaded.
const NO_SERVERS_FOLDER: &str =
    "the row names {servers}, and a copy that was not downloaded has no such folder";

/// The variables of a row for `candidate` of `key`, served from `root`.
fn expander(core: &Core, key: &ServerKey, root: &Path, candidate: &Candidate) -> Expander {
    let worktree = core
        .folder_of(&key.workspace)
        .unwrap_or_else(|| root.to_path_buf());
    let side = match key.side {
        Side::New => "new",
        Side::Old { .. } => "old",
    };
    let servers = (candidate.kind == CandidateKind::Downloaded)
        .then(|| {
            let server = core
                .registry
                .row(&key.language)?
                .servers
                .get(candidate.server)?;
            let installed = installed(&core.config.servers, &server.name)?;
            Some(
                core.config
                    .servers
                    .join(&server.name)
                    .join(installed.version),
            )
        })
        .flatten();
    Expander {
        vars: Vars {
            root: root.display().to_string(),
            worktree: worktree.display().to_string(),
            target: core
                .config
                .cache
                .join("targets")
                .join(key.workspace.as_str())
                .join(side)
                .display()
                .to_string(),
            platform: core
                .config
                .platform
                .map(|platform| platform.name().to_owned())
                .unwrap_or_default(),
            servers: servers
                .as_ref()
                .map(|folder| folder.display().to_string())
                .unwrap_or_default(),
            env: Some(core.config.environment.clone()),
        },
        servers: servers.is_some(),
    }
}

/// The [`Launch`] of `candidate` for `key`, every variable of its row expanded; an error
/// for a variable the row names that has no value.
pub(crate) fn launch(
    core: &Core,
    key: &ServerKey,
    root: &Path,
    candidate: &Candidate,
) -> Result<Launch, String> {
    let (row, server) = server_of(core, key, candidate)?;
    let expander = expander(core, key, root, candidate);
    let fault = |reason: String| {
        format!(
            "{} cannot be started: {reason}",
            candidate.program.display()
        )
    };

    let args = candidate
        .args
        .iter()
        .map(|arg| expander.text(arg))
        .collect::<Result<Vec<_>, _>>()
        .map_err(fault)?;

    // The server's own variables, then an old side's, each winning over what came before.
    let old_side: &[(String, String)] = match key.side {
        Side::New => &[],
        Side::Old { .. } => &row.old_side.environment,
    };
    let mut environment: Vec<(OsString, OsString)> = core.config.environment.clone();
    for (name, value) in server.environment.iter().chain(old_side) {
        let value = OsString::from(expander.text(value).map_err(fault)?);
        match environment.iter_mut().find(|slot| slot.0 == name.as_str()) {
            Some(slot) => slot.1 = value,
            None => environment.push((OsString::from(name), value)),
        }
    }

    Ok(Launch {
        program: candidate.program.clone(),
        args,
        root: root.to_path_buf(),
        environment,
        capabilities: expander.json(&server.capabilities).map_err(fault)?,
        initialization_options: server
            .initialization_options
            .as_ref()
            .map(|options| expander.json(options))
            .transpose()
            .map_err(fault)?,
        settings: expander.json(&server.settings).map_err(fault)?,
        ready: server.ready.clone(),
        language_ids: row.extensions.clone(),
        log: Some(
            core.own_cache
                .join("logs")
                .join(format!("{}.log", core.life.started + 1)),
        ),
    })
}
