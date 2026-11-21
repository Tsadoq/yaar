//! The Navigator's thread: one loop that takes every event from one channel, one at a time,
//! and hands each to the module that owns it.
//!
//! | Event | Handled by |
//! | --- | --- |
//! | [`Ask::Workspaces`], [`Ask::OnScreen`], [`Ask::Consents`] | `lifecycle.rs` |
//! | [`Ask::Download`], [`Ask::Update`], [`Event::Downloaded`] | `fetch.rs` |
//! | [`Ask::OldSide`], [`Ask::Exported`] | `old_side.rs` |
//! | [`Ask::Hover`], [`Ask::Definition`], [`Ask::References`], [`Ask::Open`], [`Ask::Cancel`], [`Ask::FilesChanged`] | `answers.rs` |
//! | [`Event::Server`], [`Event::Trial`] | `lifecycle.rs`, which hands an answer of a server to `answers.rs` |
//! | [`Event::Found`] | `answers.rs` |
//! | [`Event::Tick`], and before every event | `lifecycle::look_at_the_clock` |
//!
//! Nothing a handler does waits: a server, a trial run, a download and a text search run
//! on threads of their own and send their outcome to [`Core::events`]. When the asks end
//! (every [`crate::Port`] dropped), the loop ends: `lifecycle::close` stops every server and
//! waits for each to be gone, then `old_side::close` removes every export and the Navigator's
//! own cache, so that no export is removed under a process that reads it.
//!
//! [`Core`] is the state every module reads; each module keeps what is its own in its own
//! part of it ([`Core::life`], [`Core::fetch`], [`Core::answers`], [`Core::old`]), whose fields only that
//! module touches. A module reaches another only through the `pub(crate)` functions the
//! other's doc lists.

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

/// Which files a server serves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Side {
    /// The files on disk of the worktree.
    New,
    /// The export of a version: the old side of a change.
    Old {
        /// The version, by its full id.
        version: String,
    },
}

/// Names one server the Navigator runs or looks for: of one workspace, one row, one side.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ServerKey {
    /// The workspace.
    pub(crate) workspace: WorkspaceId,
    /// The row, by its name.
    pub(crate) language: String,
    /// Which files.
    pub(crate) side: Side,
}

/// Everything that reaches the Navigator's thread.
pub(crate) enum Event {
    /// What the Frame's model handed it.
    Ask(Ask),
    /// Every sender of the asks is gone.
    Closed,
    /// What a server's reading thread told. `generation` names the start of the server
    /// the event is of, so that an event of a server already replaced is known as such.
    Server {
        /// The server.
        key: ServerKey,
        /// Which start of it.
        generation: u64,
        /// What happened.
        event: ServerEvent,
    },
    /// A trial run ended.
    Trial {
        /// The server it was for.
        key: ServerKey,
        /// Which search of it.
        generation: u64,
        /// The candidate tried.
        candidate: Candidate,
        /// Whether it ended with success within [`crate::TRIAL_DEADLINE`].
        passed: bool,
    },
    /// A download ended.
    Downloaded {
        /// The row, by its name.
        language: String,
        /// What is installed now, or why not.
        outcome: Result<Installed, DownloadError>,
    },
    /// A text search ended.
    Found {
        /// The ask it answers.
        id: AskId,
        /// Every match, or why the search failed.
        found: Result<Vec<Found>, String>,
    },
    /// Time passed: [`LOOK_EVERY`].
    Tick,
}

/// The state of the Navigator's thread.
pub(crate) struct Core {
    /// What it was started with.
    pub(crate) config: NavigatorConfig,
    /// The rows read at its start.
    pub(crate) registry: Registry,
    /// Where what it tells goes.
    pub(crate) told: async_channel::Sender<Told>,
    /// Where the threads it starts send their outcomes.
    pub(crate) events: Sender<Event>,
    /// Every workspace, as last told.
    pub(crate) workspaces: Vec<WorkspaceRoot>,
    /// The workspace on screen, as last told.
    pub(crate) on_screen: Option<WorkspaceId>,
    /// Every answer on the servers inside a repository, as last told.
    pub(crate) consents: Vec<RepositoryConsent>,
    /// Where this Navigator's own cache lies: `<cache>/navigator/<pid>-<k>` ([`own_cache`]).
    pub(crate) own_cache: PathBuf,
    /// What `lifecycle.rs` keeps.
    pub(crate) life: lifecycle::Life,
    /// What `fetch.rs` keeps.
    pub(crate) fetch: fetch::Fetches,
    /// What `answers.rs` keeps.
    pub(crate) answers: answers::Answers,
    /// What `old_side.rs` keeps.
    pub(crate) old: old_side::OldSides,
}

impl Core {
    /// Tells the Frame's model `told`. A model that is gone hears nothing, and the asks end
    /// soon after.
    pub(crate) fn tell(&self, told: Told) {
        let _ = self.told.try_send(told);
    }

    /// The root of `workspace`: its worktree's folder, when it is known and there.
    pub(crate) fn folder_of(&self, workspace: &WorkspaceId) -> Option<PathBuf> {
        self.workspaces
            .iter()
            .find(|root| &root.id == workspace)
            .and_then(|root| root.folder.clone())
    }
}

/// Counts the Navigators started in this process, for [`own_cache`].
static STARTED: AtomicU64 = AtomicU64::new(1);

/// The folder of a new Navigator's own cache: `<cache>/navigator/<pid>-<k>` (`cache` is
/// `<cache>/navigator`, as `NavigatorConfig::cache` holds it), `<pid>` this process and
/// `<k>` counting the Navigators this process started, from 1, so that two Navigators of one
/// process (two windows, or two in one test) never share their exports or their logs. At
/// its start a Navigator removes every such folder whose `<pid>`, the part before the `-`,
/// names no process that runs.
pub(crate) fn own_cache(cache: &std::path::Path) -> PathBuf {
    let k = STARTED.fetch_add(1, Ordering::Relaxed);
    cache.join(format!("{}-{k}", std::process::id()))
}

/// The Navigator's thread: reads the registry, then takes events until the asks end.
pub(crate) fn run(config: NavigatorConfig, end: PortEnd) {
    let (events, inbox): (Sender<Event>, Receiver<Event>) = mpsc::channel();

    // The asks, moved onto the one channel, then the news that they ended.
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

    // The clock is looked at after every event, and at least this often.
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
        // The clock first: whatever the event is, it is handled on servers that the time
        // passed has already stopped, and a test that moved its clock and then asks
        // something reads, in what the ask makes the Navigator tell, the stop or its absence.
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
            Event::Tick => {}
        }
    }

    // The servers first, each reaped: an export is removed only once no process reads it.
    lifecycle::close(&mut core);
    old_side::close(&mut core);
}

/// Hands an ask to the module that owns it.
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
