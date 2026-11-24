//! Answers what a name in a diff is from its file's language server; text match until ready.
//! Folders: `<data>/language-servers/<server>/<version>/` downloads, `<cache>/navigator/<pid>-<k>/`
//! exports and logs, `<cache>/navigator/targets/<workspace>/<side>/` builds.

mod answers;
mod clock;
pub mod download;
pub mod driver;
mod fetch;
mod lifecycle;
pub mod lsp;
mod navigator;
mod old_side;
pub mod port;
pub mod readiness;
pub mod registry;
pub mod search;
pub mod shell;

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
pub use shell::{LoginShell, SHELL_DEADLINE};

pub const STOP_AFTER: Duration = Duration::from_secs(10 * 60);

pub const LOOK_EVERY: Duration = Duration::from_secs(1);

pub const TRIAL_DEADLINE: Duration = Duration::from_secs(10);

pub const STOP_DEADLINE: Duration = Duration::from_secs(2);

pub const CACHE_FOLDER: &str = "navigator";

pub const SERVERS_FOLDER: &str = "language-servers";

pub struct NavigatorConfig {
    pub builtin: String,
    pub user_file: PathBuf,
    pub servers: PathBuf,
    pub cache: PathBuf,
    /// On macOS a Window started from the Finder or the Dock has the system's short search path.
    pub search_path: Vec<PathBuf>,
    /// A server gets these variables and no other, plus its server's `environment`.
    pub environment: Vec<(OsString, OsString)>,
    /// The login shell whose environment, taken once at the first search, fills in what
    /// `environment` lacks; the merged `PATH` becomes `search_path`. None keeps both fields
    /// as given, as a test with its own environment does.
    pub login_shell: Option<LoginShell>,
    pub platform: Option<Platform>,
    pub stop_after: Duration,
    pub clock: Arc<dyn Clock>,
}

impl NavigatorConfig {
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
            login_shell: Some(LoginShell {
                program: yaar_link::user_shell(),
                deadline: SHELL_DEADLINE,
            }),
            platform: Platform::current(),
            stop_after: STOP_AFTER,
            clock: Arc::new(SystemClock),
        }
    }
}

pub struct Navigator {
    thread: Option<JoinHandle<()>>,
}

impl Navigator {
    pub fn start(config: NavigatorConfig, end: PortEnd) -> Navigator {
        let thread = std::thread::Builder::new()
            .name("navigator".to_owned())
            .spawn(move || navigator::run(config, end))
            .expect("start the navigator's thread");
        Navigator {
            thread: Some(thread),
        }
    }

    /// Gives whether the thread ended within `within`.
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
