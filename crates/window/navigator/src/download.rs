//! The downloader: fetches the server a row pins, checks its digest, and unpacks it into the
//! app's folder (FR-058, ADR-015 point 7). A download belongs to a server of a row, and is
//! kept under the server's name: `<servers>/<server>/<version>/`.
//!
//! # What is sent
//!
//! One plain `GET` of the row's address, following at most five redirects, each to an
//! `https` address. The request carries no account, no token, no cookie and no header that
//! names the user, the machine or a path: `Host`, `User-Agent` [`USER_AGENT`] and `Accept`,
//! and nothing else. Nothing is uploaded. TLS is rustls with ring, checked against the
//! certificates the system trusts.
//!
//! An address is fetched when it is `https://`, or `http://` to a loopback host
//! (`127.0.0.1`, `[::1]` or `localhost`); any other is refused with
//! [`DownloadError::NotAllowed`] before anything is sent. The digest guards what arrives
//! either way; the loopback address is how the tests serve a download from a listener of
//! their own, through this same code.
//!
//! # What is written
//!
//! The bytes are hashed as they stream into a file `<servers>/.partial-<server>-<n>`; a
//! digest that differs from the row's removes that file and refuses the download
//! ([`DownloadError::Digest`]) with nothing unpacked (LSP-05). A matching one is unpacked
//! into `<servers>/<server>/.partial-<version>`, its program made executable, the folder
//! renamed to `<servers>/<server>/<version>`, and then `<servers>/<server>/installed.toml`
//! written, through a file renamed into place, naming the version, the digest and the
//! program: from then on the row's downloaded candidate is that version ([`installed`]).
//! The version that was installed before is removed after that. A failure at any step
//! leaves what was installed before as it was.
//!
//! An entry of a tar file whose path, once `strip` parts are dropped, is absolute or climbs
//! out of the folder, or that is a link, is refused: nothing outside the folder is written.
//!
//! Nothing here decides when to download: the Navigator calls [`Downloader::fetch`] only
//! once the user accepted an offer or asked for an update, never for a registered row.

use std::path::{Path, PathBuf};

use crate::registry::{Download, Platform, Registry};

/// The user agent of every request: the app's name, and nothing of the user or the machine.
pub const USER_AGENT: &str = "yaar-review";

/// The file, in a server's folder, that names the version in use: a TOML table of three
/// strings, `version`, `sha256`, and `program`, the program's path relative to the
/// version's folder `<servers>/<row>/<version>`, as the row's `program` names it.
///
/// ```toml
/// version = "2026-11-09"
/// sha256 = "e43a65008f8f4291320f33c0545dd577ad7623a7407b584a27155833d3722dfb"
/// program = "rust-analyzer"
/// ```
pub const INSTALLED_FILE: &str = "installed.toml";

/// Why a download failed, as the user is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// The address is neither `https://` nor `http://` to a loopback host.
    NotAllowed(String),
    /// The server answered with this status.
    Status(u16),
    /// No answer, or one cut short: the reason.
    Network(String),
    /// The bytes fetched are not the ones the row pins.
    Digest {
        /// The row's digest.
        expected: String,
        /// The digest of what arrived.
        found: String,
    },
    /// The archive could not be unpacked: the reason.
    Unpack(String),
    /// A file of the app's folder could not be written: the reason.
    Folder(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAllowed(url) => write!(f, "{url} is not an address the app downloads from"),
            Self::Status(status) => write!(f, "the server answered {status}"),
            Self::Network(reason) => write!(f, "the download failed: {reason}"),
            Self::Digest { expected, found } => write!(
                f,
                "the download is not the one pinned: its digest is {found}, not {expected}"
            ),
            Self::Unpack(reason) => write!(f, "the download could not be unpacked: {reason}"),
            Self::Folder(reason) => write!(f, "the app's folder could not be written: {reason}"),
        }
    }
}

impl std::error::Error for DownloadError {}

/// A server the app downloaded and uses, as `installed.toml` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The version.
    pub version: String,
    /// The digest of the bytes it was unpacked from.
    pub sha256: String,
    /// Its program, by its absolute path.
    pub program: PathBuf,
}

/// The server the app downloaded for the server named `server` into `servers` and uses,
/// when `installed.toml` names one whose program is there.
pub fn installed(servers: &Path, server: &str) -> Option<Installed> {
    let _ = (servers, server);
    todo!()
}

/// Whether `url` is an address the downloader fetches from: `https://`, or `http://` to a
/// loopback host.
pub fn allowed(url: &str) -> bool {
    let _ = url;
    todo!()
}

/// The downloader. One is enough for every download of a process.
pub struct Downloader {
    agent: ureq::Agent,
}

impl Downloader {
    /// A downloader with the certificates the system trusts. An error, as a sentence, when
    /// none could be read.
    pub fn new() -> Result<Downloader, String> {
        todo!()
    }

    /// Fetches `download` of the server named `server` into `servers`, as the module doc
    /// says, and gives what is installed now.
    pub fn fetch(
        &self,
        server: &str,
        download: &Download,
        servers: &Path,
    ) -> Result<Installed, DownloadError> {
        let _ = (&self.agent, server, download, servers);
        todo!()
    }
}

/// For each server of each row of `registry` with a download for `platform`: nothing when
/// `servers` holds that version with that digest already, else [`Downloader::fetch`].
/// `told` hears one line for each server, saying which it was. What `cargo xtask language-servers` runs,
/// and nothing else does.
pub fn fill(
    registry: &Registry,
    platform: Platform,
    servers: &Path,
    told: &mut dyn FnMut(&str),
) -> Result<(), DownloadError> {
    let _ = (registry, platform, servers, told);
    todo!()
}

/// The names of the servers of `registry` with a download for `platform` whose pinned
/// version `servers` does not hold with its digest: what a test that needs the real servers
/// says is missing. Empty when the folder is complete.
pub fn missing(registry: &Registry, platform: Platform, servers: &Path) -> Vec<String> {
    let _ = (registry, platform, servers);
    todo!()
}
