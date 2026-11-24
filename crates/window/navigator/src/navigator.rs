//! One loop takes every event from one channel; nothing a handler does waits.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use yaar_link::WorkspaceId;
use yaar_text_search::Found;

use crate::download::{DownloadError, Installed};
use crate::driver::ServerEvent;
use crate::port::{Ask, AskId, PortEnd, RepositoryConsent, Told, WorkspaceRoot};
use crate::registry::Registry;
use crate::search::Candidate;
use crate::{LOOK_EVERY, NavigatorConfig, answers, fetch, lifecycle, old_side};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Side {
    New,
    Old { version: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ServerKey {
    pub(crate) workspace: WorkspaceId,
    pub(crate) language: String,
    pub(crate) side: Side,
}

pub(crate) enum Event {
    Ask(Ask),
    Closed,
    /// `generation` names the start of the server, so an event of a replaced server is recognised.
    Server {
        key: ServerKey,
        generation: u64,
        event: ServerEvent,
    },
    Trial {
        key: ServerKey,
        generation: u64,
        candidate: Candidate,
        passed: bool,
    },
    Downloaded {
        language: String,
        outcome: Result<Installed, DownloadError>,
    },
    Found {
        id: AskId,
        found: Result<Vec<Found>, String>,
    },
    Shell(Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, crate::shell::ShellError>),
    Tick,
}

pub(crate) struct Core {
    pub(crate) config: NavigatorConfig,
    pub(crate) registry: Registry,
    pub(crate) told: async_channel::Sender<Told>,
    pub(crate) events: Sender<Event>,
    pub(crate) workspaces: Vec<WorkspaceRoot>,
    pub(crate) on_screen: Option<WorkspaceId>,
    pub(crate) consents: Vec<RepositoryConsent>,
    pub(crate) own_cache: PathBuf,
    pub(crate) life: lifecycle::Life,
    pub(crate) fetch: fetch::Fetches,
    pub(crate) answers: answers::Answers,
    pub(crate) old: old_side::OldSides,
}

impl Core {
    /// A model that is gone hears nothing.
    pub(crate) fn tell(&self, told: Told) {
        let _ = self.told.try_send(told);
    }

    pub(crate) fn folder_of(&self, workspace: &WorkspaceId) -> Option<PathBuf> {
        self.workspaces
            .iter()
            .find(|root| &root.id == workspace)
            .and_then(|root| root.folder.clone())
    }
}

static STARTED: AtomicU64 = AtomicU64::new(1);

/// `<cache>/navigator/<pid>-<k>`, `k` counting this process's Navigators from 1 so that two never
/// share exports or logs.
pub(crate) fn own_cache(cache: &std::path::Path) -> PathBuf {
    let k = STARTED.fetch_add(1, Ordering::Relaxed);
    cache.join(format!("{}-{k}", std::process::id()))
}

pub(crate) fn run(config: NavigatorConfig, end: PortEnd) {
    let (events, inbox): (Sender<Event>, Receiver<Event>) = mpsc::channel();

    let asks = end.asks;
    let to_loop = events.clone();
    std::thread::Builder::new()
        .name("navigator-asks".to_owned())
        .spawn(move || {
            while let Ok(ask) = asks.recv_blocking() {
                if to_loop.send(Event::Ask(ask)).is_err() {
                    return;
                }
            }
            let _ = to_loop.send(Event::Closed);
        })
        .expect("start the navigator's reader of asks");

    let to_loop = events.clone();
    std::thread::Builder::new()
        .name("navigator-clock".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(LOOK_EVERY);
                if to_loop.send(Event::Tick).is_err() {
                    return;
                }
            }
        })
        .expect("start the navigator's clock");

    let own_cache = own_cache(&config.cache);
    let mut core = Core {
        registry: Registry::default(),
        told: end.told,
        events,
        workspaces: Vec::new(),
        on_screen: None,
        consents: Vec::new(),
        own_cache,
        life: lifecycle::Life::default(),
        fetch: fetch::Fetches::default(),
        answers: answers::Answers::default(),
        old: old_side::OldSides::default(),
        config,
    };
    lifecycle::started(&mut core);

    while let Ok(event) = inbox.recv() {
        // The clock first: the event is handled on servers that the time passed has already
        // stopped.
        lifecycle::look_at_the_clock(&mut core);
        match event {
            Event::Closed => break,
            Event::Ask(ask) => dispatch(&mut core, ask),
            Event::Server {
                key,
                generation,
                event,
            } => lifecycle::server_event(&mut core, key, generation, event),
            Event::Trial {
                key,
                generation,
                candidate,
                passed,
            } => lifecycle::trial_ended(&mut core, key, generation, candidate, passed),
            Event::Downloaded { language, outcome } => {
                fetch::downloaded(&mut core, language, outcome)
            }
            Event::Found { id, found } => answers::found(&mut core, id, found),
            Event::Shell(taken) => lifecycle::shell_taken(&mut core, taken),
            Event::Tick => {}
        }
    }

    // The servers first, each reaped: an export is removed only once no process reads it.
    lifecycle::close(&mut core);
    old_side::close(&mut core);
}

fn dispatch(core: &mut Core, ask: Ask) {
    match ask {
        Ask::Workspaces(roots) => {
            // The servers of a workspace gone are stopped first, then its export removed.
            lifecycle::workspaces(core, roots);
            old_side::workspaces_changed(core);
        }
        Ask::OnScreen(workspace) => lifecycle::on_screen(core, workspace),
        Ask::Consents(consents) => lifecycle::consents(core, consents),
        Ask::Download { language } => fetch::download(core, &language),
        Ask::Update { language } => fetch::update(core, &language),
        Ask::OldSide { workspace, version } => old_side::shown(core, workspace, version),
        Ask::Exported { asked, failed } => old_side::exported(core, asked, failed),
        Ask::StopAfter(after) => core.config.stop_after = after,
        Ask::FilesChanged { workspace, paths } => answers::files_changed(core, &workspace, &paths),
        Ask::Hover { id, place } => answers::ask(core, id, answers::Question::Hover, place),
        Ask::Definition { id, place } => {
            answers::ask(core, id, answers::Question::Definition, place)
        }
        Ask::References { id, place } => {
            answers::ask(core, id, answers::Question::References, place)
        }
        Ask::Open { id, location } => answers::open(core, id, location),
        Ask::Cancel { id } => answers::cancel(core, id),
    }
}
