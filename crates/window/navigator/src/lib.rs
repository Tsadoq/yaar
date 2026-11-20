//! Navigator (`W-LSP`).
//!
//! Owns: the registry of language servers, the driver, the downloader, the old-side module,
//! readiness, starting and stopping servers.
//!
//! The Navigator answers what a name in a diff is, where it is defined and who uses it,
//! from the language server of its file, on new and on old code. It runs in the Window's
//! process, on a thread of its own, and its servers are the Window's children: they end
//! with it. It holds no database and runs no git: what it needs of the Keeper, the old side
//! of a change written into a folder, it asks for through the Frame.
//!
//! Surface:
//! - [`port`], the vocabulary between the Window and the Navigator: the [`Ask`]s the
//!   Frame's model hands it and what it tells back, [`Told`], through the two ends
//!   [`Port`] and [`PortEnd`];
//! - [`Navigator`], started with a [`NavigatorConfig`] on a [`PortEnd`]: the thread that
//!   finds, starts, asks and stops the servers. [`NavigatorConfig::for_this_user`] is what
//!   the Window hands it;
//! - [`Clock`] and [`SystemClock`], the time idle servers are stopped by;
//! - [`registry`]: the one TOML format of the built-in table [`BUILTIN`] and of the user's
//!   file [`USER_FILE`], read row by row ([`Registry::read`]), and the [`Platform`]s a row
//!   can download for;
//! - [`search`]: which servers a row finds on this machine and in which order they are
//!   tried ([`search::order`]), and what is shown when none is found;
//! - [`readiness`]: the three rules by which a server's answers count as sound;
//! - [`driver`] and [`lsp`]: one server, started from a row and spoken to in the Language
//!   Server Protocol, with no code for a particular language or server;
//! - [`download`]: the downloader, which fetches the server a row pins, checks its digest
//!   and unpacks it into the app's folder; the same code fills the folder of real servers
//!   the tests use (`cargo xtask language-servers`).
//!
//! # What runs where
//!
//! [`Navigator::start`] spawns the Navigator's thread, which takes the asks and the
//! events of its servers, its trials, its downloads and its searches one at a time from
//! one channel (`navigator.rs`); nothing it does waits for a server, a download or a
//! search, which run on threads of their own and report back on that channel. The thread
//! that draws never waits for the Navigator: the Frame's model awaits answers on the
//! channel of [`Port::told`].
//!
//! # Which server, and when
//!
//! For the workspace on screen the Navigator starts a server of each row whose `markers`
//! lie at the worktree's root, and of any other row at the first ask about one of its
//! files (FR-064). The candidates of a row, copies of its servers, are tried in the order
//! of FR-060: the one the user registered, the project's own, one installed on the
//! machine, one the app downloaded, each kind in the order the row names its servers
//! ([`search::order`]). A candidate that lies inside a repository whose answer
//! the Navigator was not told is passed over and the user asked (FR-062); one refused is
//! passed over for good. Each candidate left is run once with its server's `trial`
//! arguments, and passed over unless it ends with success (FR-061); the one chosen that
//! then fails to start, or dies, is passed over too, and the next tried. With none left
//! the state is missing, with the offer to download when the row has a download for this
//! machine and is not the user's (FR-057, FR-073). A download happens only when the user
//! accepts the offer ([`Ask::Download`]) and an update only when the user asks
//! ([`Ask::Update`], FR-063).
//!
//! Until a server's readiness rule is met, and for a file of a language no row knows, the
//! Navigator answers by text match, with `yaar-text-search`, and marks every such answer
//! (FR-068).
//!
//! A workspace off screen for [`NavigatorConfig::stop_after`] has every server of its own
//! stopped, those of its old side included; back on screen they start again (FR-065,
//! FR-066).
//!
//! # The old side of a diff
//!
//! A place on the old side of a change ([`port::Tree::Version`]) is answered by a second
//! server of the same row, started at the first such ask on the export of that version:
//! the Navigator asks for the export ([`Told::ExportWanted`]) into a folder of its own
//! cache, links into it what the row's `old_side.link` names from the worktree, and starts
//! the server there. One export a workspace: another version shown replaces it
//! ([`Ask::OldSide`]); a workspace closed loses it.
//!
//! # The app's folders
//!
//! | Folder | What it holds |
//! | --- | --- |
//! | `<data>/language-servers.toml` | the user's registry, [`USER_FILE`] |
//! | `<data>/language-servers/<server>/<version>/` | a server the app downloaded; `<data>/language-servers/<server>/installed.toml` names the version in use |
//! | `<cache>/navigator/<pid>/old/<n>/` | an export of an old side, while it is used |
//! | `<cache>/navigator/<pid>/logs/<n>.log` | what a server wrote on its standard error |
//! | `<cache>/navigator/targets/<workspace>/<side>/` | the build folder a row's `{target}` names, `new` or `old`, kept between runs |
//!
//! `<data>` is `yaar_link::data_dir()` and `<cache>` `yaar_link::cache_dir()`; `<pid>` the
//! Window's process. At its start the Navigator removes every `<cache>/navigator/<pid>`
//! whose process no longer runs. Nothing the Navigator, or a server it starts from a
//! built-in row, writes lies in a worktree (NFR-021).

mod answers;
mod clock;
pub mod download;
pub mod driver;
mod lifecycle;
pub mod lsp;
mod navigator;
mod old_side;
pub mod port;
pub mod readiness;
pub mod registry;
pub mod search;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use clock::{Clock, SystemClock};
pub use port::{
    Answered, Ask, AskId, ExportAsk, FileKind, Language, Location, Offer, OpenedFile, Place, Port,
    PortEnd, RepositoryConsent, ServerState, State, Told, Tree, WorkspaceRoot, port,
};
pub use registry::{BUILTIN, Platform, Registry, USER_FILE};

/// How long a workspace stays off screen before its servers stop, when the user set no
/// other time: ten minutes (FR-065).
pub const STOP_AFTER: Duration = Duration::from_secs(10 * 60);

/// How often, at the least, the Navigator looks at its clock when nothing else happens.
pub const LOOK_EVERY: Duration = Duration::from_secs(1);

/// How long a candidate's trial run may take before it counts as failed.
pub const TRIAL_DEADLINE: Duration = Duration::from_secs(10);

/// How long a server is given to end after it was asked to, before it is killed.
pub const STOP_DEADLINE: Duration = Duration::from_secs(2);

/// The Navigator's folder in `yaar_link::cache_dir()`.
pub const CACHE_FOLDER: &str = "navigator";

/// The folder, in `yaar_link::data_dir()`, of the servers the app downloads.
pub const SERVERS_FOLDER: &str = "language-servers";

/// What the Navigator is started with: where its folders lie, the registry, the machine it
/// runs on and the environment its servers run in. The Window hands it
/// [`NavigatorConfig::for_this_user`]; a test hands it folders, a registry and an
/// environment of its own.
pub struct NavigatorConfig {
    /// The text of the built-in table: [`BUILTIN`].
    pub builtin: String,
    /// The user's registry, [`USER_FILE`] in the app's data folder. A file that is not
    /// there holds no row.
    pub user_file: PathBuf,
    /// Where the app's downloads lie: `<data>/language-servers`.
    pub servers: PathBuf,
    /// The Navigator's cache folder: `<cache>/navigator`.
    pub cache: PathBuf,
    /// The folders a server "installed on the machine" is looked for in, in order: the
    /// Window's own search path.
    pub search_path: Vec<PathBuf>,
    /// The whole environment every server, and every trial run, runs in: the Window's own.
    /// A server gets these variables and no other, with its server's `environment` added.
    pub environment: Vec<(OsString, OsString)>,
    /// The machine, for the downloads: none where no row can download for it.
    pub platform: Option<Platform>,
    /// How long a workspace stays off screen before its servers stop: the user's setting,
    /// or [`STOP_AFTER`].
    pub stop_after: Duration,
    /// The time idle servers are stopped by.
    pub clock: Arc<dyn Clock>,
}

impl NavigatorConfig {
    /// What the Window hands the Navigator: [`BUILTIN`], the user's file and the downloads
    /// in `yaar_link::data_dir()`, the cache in `yaar_link::cache_dir()`, the search path
    /// and the whole environment of this process, [`Platform::current`], [`STOP_AFTER`]
    /// and [`SystemClock`].
    pub fn for_this_user() -> NavigatorConfig {
        let data = yaar_link::data_dir();
        NavigatorConfig {
            builtin: BUILTIN.to_owned(),
            user_file: data.join(USER_FILE),
            servers: data.join(SERVERS_FOLDER),
            cache: yaar_link::cache_dir().join(CACHE_FOLDER),
            search_path: std::env::var_os("PATH")
                .map(|search| std::env::split_paths(&search).collect())
                .unwrap_or_default(),
            environment: std::env::vars_os().collect(),
            platform: Platform::current(),
            stop_after: STOP_AFTER,
            clock: Arc::new(SystemClock),
        }
    }
}

/// The Navigator's thread. It ends, its servers stopped and its exports removed, once
/// every [`Port::asks`] sender is gone.
pub struct Navigator {
    thread: Option<JoinHandle<()>>,
}

impl Navigator {
    /// Starts the Navigator on `end`: it reads the registry, tells [`Told::Languages`],
    /// removes the cache folders of Windows that no longer run, and waits for asks.
    pub fn start(config: NavigatorConfig, end: PortEnd) -> Navigator {
        let thread = std::thread::Builder::new()
            .name("navigator".to_owned())
            .spawn(move || navigator::run(config, end))
            .expect("start the navigator's thread");
        Navigator {
            thread: Some(thread),
        }
    }

    /// Waits, up to `within`, for the Navigator to end: for the [`Port`] to be dropped and
    /// every server to be gone. Gives whether it ended. A repeated look at one fact that
    /// only becomes true: the thread's end.
    pub fn wait(mut self, within: Duration) -> bool {
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let deadline = Instant::now() + within;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                self.thread = Some(thread);
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = thread.join();
        true
    }
}
