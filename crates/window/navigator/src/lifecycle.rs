//! Which servers run, and when: `ensure` takes a `ServerKey` through search, trial and start.
//! Every stop reaps the process (and the trial run) before it returns, so a published
//! `Told::Servers` names no process that still runs.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use yaar_link::{RepositoryId, WorkspaceId};

use crate::download::{Installed, installed};
use crate::driver::{Launch, Server, ServerEvent, end_with_the_window};
use crate::navigator::{Core, Event, ServerKey, Side};
use crate::port::{Language, Offer, RepositoryConsent, ServerState, State, Told, WorkspaceRoot};
use crate::registry::{Registry, Row, Source, Vars, expand, expand_json};
use crate::search::{Candidate, CandidateKind, inside, look, order, when_none};
use crate::{STOP_DEADLINE, TRIAL_DEADLINE, answers};

#[derive(Default)]
pub(crate) struct Life {
    keys: BTreeMap<ServerKey, Keyed>,
    left: HashMap<WorkspaceId, Instant>,
    started: u64,
    /// One count for every key, so that an event of a stopped key is never taken for one of the key
    /// made again in its place.
    generation: u64,
    shell: ShellTaken,
    held: Vec<ServerKey>,
    shell_group: crate::shell::Group,
}

#[derive(Default, PartialEq, Eq)]
enum ShellTaken {
    #[default]
    NotYet,
    Running,
    Taken,
}

struct Keyed {
    root: PathBuf,
    generation: u64,
    left: VecDeque<Candidate>,
    phase: Phase,
    question: Option<(RepositoryId, PathBuf)>,
    /// Whether this search passed over a candidate for the repository's answer: a yes searches
    /// the key again, since that candidate comes before the one that runs.
    passed_over: bool,
}

struct Running {
    candidate: Candidate,
    server: Server,
    version: Option<String>,
}

enum Phase {
    Trying {
        candidate: Candidate,
        process: Arc<Mutex<Option<Child>>>,
    },
    Starting {
        running: Running,
    },
    Ready {
        running: Running,
    },
    NoneLeft,
    Downloading {
        offer: Offer,
    },
    Held,
}

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
        // A number too large for a process id, and 0, which names a group and not a process, are no
        // process of ours.
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

fn process_runs(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only asks whether the process exists; it sends nothing.
    let signalled = unsafe { libc::kill(pid, 0) };
    // A process of another user exists, and answers that it may not be signalled.
    signalled == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

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
                    // A workspace whose folder moved is another place. An old side serves an
                    // export, which `old_side.rs` keeps.
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

    if let Some(on_screen) = core.on_screen.clone() {
        ensure_by_markers(core, &on_screen);
    }
    publish(core);
}

/// Ends with `publish` also when nothing changed: a test that moved its clock reads what it tells.
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
                // The candidate passed over comes before the one that runs, or the key has none:
                // from the start again.
                Some(true) if core.life.keys[&key].passed_over => {
                    restart(core, &key);
                    changed = true;
                }
                Some(false) => {
                    let cleared = core
                        .life
                        .keys
                        .get_mut(&key)
                        .is_some_and(|keyed| keyed.question.take().is_some());
                    if cleared {
                        if matches!(
                            core.life.keys.get(&key).map(|keyed| &keyed.phase),
                            Some(Phase::NoneLeft)
                        ) {
                            crate::fetch::maybe_auto_download(core, &key.language);
                        }
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
                other => {
                    keyed.phase = other;
                    return;
                }
            };
            // Only a ready server was sent a request, so only its exit can leave an ask waiting.
            if was_ready {
                answers::server_gone(core, &key);
            }
            let Some(mut keyed) = core.life.keys.remove(&key) else {
                return;
            };
            advance(core, &key, &mut keyed);
            let none_left = matches!(keyed.phase, Phase::NoneLeft);
            let language = key.language.clone();
            core.life.keys.insert(key, keyed);
            if none_left {
                crate::fetch::maybe_auto_download(core, &language);
            }
            publish(core);
        }
    }
}

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
    let mut none_left = false;
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
        none_left = matches!(keyed.phase, Phase::NoneLeft);
    }
    let language = key.language.clone();
    core.life.keys.insert(key, keyed);
    if current {
        if none_left {
            crate::fetch::maybe_auto_download(core, &language);
        }
        publish(core);
    }
}

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

pub(crate) fn shell_taken(
    core: &mut Core,
    taken: Result<Vec<(OsString, OsString)>, crate::shell::ShellError>,
) {
    core.life.shell = ShellTaken::Taken;
    match taken {
        Ok(environment) => {
            core.config.search_path = crate::shell::search_path(&environment);
            core.config.environment = environment;
        }
        Err(why) => {
            let program = core
                .config
                .login_shell
                .as_ref()
                .map(|shell| shell.program.display().to_string())
                .unwrap_or_default();
            eprintln!("yaar-navigator: the login shell {program}: {why}");
        }
    }
    let held = std::mem::take(&mut core.life.held);
    for key in held {
        // A key stopped meanwhile is forgotten, and one made again after it is searched once, where
        // it was first held.
        if matches!(
            core.life.keys.get(&key).map(|keyed| &keyed.phase),
            Some(Phase::Held)
        ) {
            restart(core, &key);
        }
    }
    publish(core);
}

fn must_wait_for_the_shell(core: &mut Core) -> bool {
    match core.life.shell {
        ShellTaken::Taken => false,
        ShellTaken::Running => true,
        ShellTaken::NotYet => {
            let Some(shell) = core.config.login_shell.clone() else {
                return false;
            };
            let base = core.config.environment.clone();
            let events = core.events.clone();
            let group = core.life.shell_group.clone();
            let started = std::thread::Builder::new()
                .name("navigator-shell".to_owned())
                .spawn(move || {
                    let _ = events.send(Event::Shell(crate::shell::take_in(&shell, &base, &group)));
                });
            match started {
                Ok(_) => {
                    core.life.shell = ShellTaken::Running;
                    true
                }
                Err(error) => {
                    eprintln!("yaar-navigator: the login shell cannot be run: {error}");
                    core.life.shell = ShellTaken::Taken;
                    false
                }
            }
        }
    }
}

fn hold(core: &mut Core, key: &ServerKey, keyed: &mut Keyed) {
    keyed.left.clear();
    keyed.question = None;
    keyed.passed_over = false;
    keyed.phase = Phase::Held;
    core.life.held.retain(|held| held != key);
    core.life.held.push(key.clone());
}

pub(crate) fn close(core: &mut Core) {
    // Dropped first: the end of a server would answer them by text match, and no one is left to
    // read it.
    answers::forget_the_waiting(core);
    core.life.shell_group.end();
    let keys: Vec<ServerKey> = core.life.keys.keys().cloned().collect();
    for key in &keys {
        stop_quietly(core, key);
    }
    publish(core);
}

/// `root` is the worktree's folder for the new side, the export's for an old side.
pub(crate) fn ensure(core: &mut Core, key: &ServerKey, root: &Path) -> State {
    if ensure_key(core, key, root) {
        publish(core);
    }
    state_of(core, key).unwrap_or(State::Missing { offer: None })
}

fn ensure_key(core: &mut Core, key: &ServerKey, root: &Path) -> bool {
    if core.life.keys.contains_key(key) {
        return false;
    }
    // A workspace not on screen has its servers stopped after the set time, counted from the first
    // one asked for.
    if core.on_screen.as_ref() != Some(&key.workspace)
        && !core.life.left.contains_key(&key.workspace)
    {
        let now = core.config.clock.now();
        core.life.left.insert(key.workspace.clone(), now);
    }
    let mut keyed = Keyed {
        root: root.to_path_buf(),
        generation: 0,
        left: VecDeque::new(),
        phase: Phase::NoneLeft,
        question: None,
        passed_over: false,
    };
    if must_wait_for_the_shell(core) {
        hold(core, key, &mut keyed);
        core.life.keys.insert(key.clone(), keyed);
        return true;
    }
    core.life.generation += 1;
    keyed.generation = core.life.generation;
    search(core, key, &mut keyed);
    advance(core, key, &mut keyed);
    let none_left = matches!(keyed.phase, Phase::NoneLeft);
    core.life.keys.insert(key.clone(), keyed);
    if none_left {
        crate::fetch::maybe_auto_download(core, &key.language);
    }
    true
}

pub(crate) fn ready_server<'a>(core: &'a Core, key: &ServerKey) -> Option<&'a Server> {
    match &core.life.keys.get(key)?.phase {
        Phase::Ready { running } => Some(&running.server),
        _ => None,
    }
}

/// A server still starting reads the files when it loads, and the driver holds what is sent before
/// the answer to `initialize`.
pub(crate) fn running_server<'a>(core: &'a Core, key: &ServerKey) -> Option<&'a Server> {
    match &core.life.keys.get(key)?.phase {
        Phase::Starting { running } | Phase::Ready { running } => Some(&running.server),
        _ => None,
    }
}

pub(crate) fn state_of(core: &Core, key: &ServerKey) -> Option<State> {
    core.life
        .keys
        .get(key)
        .map(|keyed| shown(core, key, keyed).state)
}

pub(crate) fn stop(core: &mut Core, key: &ServerKey) {
    stop_quietly(core, key);
    publish(core);
}

fn stop_quietly(core: &mut Core, key: &ServerKey) -> bool {
    let Some(mut keyed) = core.life.keys.remove(key) else {
        return false;
    };
    core.life.held.retain(|held| held != key);
    if end_server(&mut keyed) {
        answers::server_gone(core, key);
    }
    true
}

fn end_server(keyed: &mut Keyed) -> bool {
    match std::mem::replace(&mut keyed.phase, Phase::NoneLeft) {
        Phase::Starting { running } => running.server.stop(STOP_DEADLINE),
        Phase::Ready { running } => {
            running.server.stop(STOP_DEADLINE);
            return true;
        }
        Phase::Trying { process, .. } => {
            // Taken out first, so that the thread that waits for it finds it gone and the lock is
            // not held while the process is reaped.
            let taken = process
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            kill_and_reap(taken);
        }
        Phase::NoneLeft | Phase::Downloading { .. } | Phase::Held => {}
    }
    false
}

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

pub(crate) fn updating(core: &Core, language: &str) -> Vec<(ServerKey, Offer)> {
    core.life
        .keys
        .iter()
        .filter(|(key, _)| key.language == language)
        .filter_map(|(key, keyed)| update_of(core, key, keyed).map(|offer| (key.clone(), offer)))
        .collect()
}

/// The keys whose tried, starting or running candidate is a downloaded copy of `server` in a
/// version other than the one just installed: its folder is gone or about to go, so they search
/// again.
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
                Phase::NoneLeft | Phase::Downloading { .. } | Phase::Held => return false,
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

pub(crate) fn downloading(core: &mut Core, key: &ServerKey, offer: Offer) {
    let Some(keyed) = core.life.keys.get_mut(key) else {
        return;
    };
    let was_ready = end_server(keyed);
    keyed.phase = Phase::Downloading { offer };
    // `fetch::download` hands over only keys that offer a download, which run nothing; the call
    // keeps a ready key from leaving asks waiting.
    if was_ready {
        answers::server_gone(core, key);
    }
    publish(core);
}

pub(crate) fn search_again(core: &mut Core, key: &ServerKey) {
    if core.life.keys.contains_key(key) {
        restart(core, key);
        publish(core);
    }
}

fn restart(core: &mut Core, key: &ServerKey) {
    let Some(mut keyed) = core.life.keys.remove(key) else {
        return;
    };
    if end_server(&mut keyed) {
        answers::server_gone(core, key);
    }
    if must_wait_for_the_shell(core) {
        hold(core, key, &mut keyed);
        core.life.keys.insert(key.clone(), keyed);
        return;
    }
    core.life.generation += 1;
    keyed.generation = core.life.generation;
    search(core, key, &mut keyed);
    advance(core, key, &mut keyed);
    let none_left = matches!(keyed.phase, Phase::NoneLeft);
    core.life.keys.insert(key.clone(), keyed);
    if none_left {
        crate::fetch::maybe_auto_download(core, &key.language);
    }
}

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
        Phase::NoneLeft | Phase::Downloading { .. } | Phase::Held => (None, None, None),
    };
    let state = if let Some((repository, program)) = &keyed.question {
        State::Asking {
            repository: repository.clone(),
            program: program.clone(),
        }
    } else {
        match &keyed.phase {
            Phase::Trying { .. } | Phase::Starting { .. } | Phase::Held => State::Starting,
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

fn repository_of<'a>(core: &'a Core, workspace: &WorkspaceId) -> Option<&'a RepositoryId> {
    core.workspaces
        .iter()
        .find(|root| &root.id == workspace)
        .map(|root| &root.repository)
}

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

fn kill_and_reap(child: Option<Child>) {
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn wait_for_trial(process: &Mutex<Option<Child>>) -> bool {
    let until = Instant::now() + TRIAL_DEADLINE;
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

struct Expander {
    vars: Vars,
    servers: bool,
}

impl Expander {
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

const NO_SERVERS_FOLDER: &str =
    "the row names {servers}, and a copy that was not downloaded has no such folder";

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
