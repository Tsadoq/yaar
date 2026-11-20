//! The downloader: fetches the server a row pins, checks its digest, and unpacks it into the
//! app's folder (FR-058, ADR-015 point 7). A download belongs to a server of a row, and is
//! kept under the server's name: `<servers>/<server>/<version>/`.
//!
//! # What is sent
//!
//! One plain `GET` of the row's address. The request carries no account, no token, no
//! cookie and no header that names the user, the machine or a path: `Host`, `User-Agent`
//! [`USER_AGENT`] and `Accept`, and nothing else. Nothing is uploaded. TLS is rustls with
//! ring, checked against the certificates the system trusts. No proxy is used: the agent is
//! built with `.proxy(None)`, so that a proxy variable of the environment (`HTTP_PROXY`,
//! `HTTPS_PROXY`, `ALL_PROXY`, in either case) is never read, and the request goes to the
//! publisher's address and nowhere else.
//!
//! An address is fetched when [`allowed`] holds: `https://`, or `http://` to the loopback
//! host written as an address, `127.0.0.1` or `[::1]` (never a name, which would go
//! through name resolution). Any other is refused with [`DownloadError::NotAllowed`] before
//! anything is sent. The digest guards what arrives either way; the loopback address is how
//! the tests serve a download from a listener of their own, through this same code.
//!
//! Redirects are followed here, not by the HTTP library: the agent is built with
//! `max_redirects(0)`, `max_redirects_will_error(false)` and `http_status_as_error(false)`,
//! so that a 301, 302, 303, 307 or 308 comes back as an answer. Its `Location` is resolved
//! against the address that answered (`url::Url::join`) and followed only when
//! [`redirect_allowed`] holds for the two: the next address is [`allowed`], and an answer
//! from an `https` address leads only to an `https` address. Anything else is refused with
//! [`DownloadError::NotAllowed`] naming the next address, before a byte is sent to it. At
//! most [`MAX_REDIRECTS`] are followed; one more is [`DownloadError::Network`]. A
//! redirect from a loopback `http` address to another loopback `http` address is followed,
//! as the tests' listener does; a row of the built-in table names `https` only.
//!
//! The agent waits at most [`CONNECT_WITHIN`] for a connection and at most
//! [`READ_WITHIN`] for each read of the answer, so that a stalled server ends the download
//! with [`DownloadError::Network`] instead of leaving it on its way for good.
//!
//! # What is written
//!
//! The bytes are hashed as they stream into a file `<servers>/.partial-<server>-<pid>-<n>`,
//! `<pid>` this process and `<n>` counting up in it, so that two processes downloading at
//! once never share a file. More bytes than the row's `size` stop the download at once,
//! with [`DownloadError::TooLarge`], the file removed: a wrong answer cannot fill the disk
//! before the digest is checked. A digest that differs from the row's removes that file
//! and refuses the download ([`DownloadError::Digest`]) with nothing unpacked (LSP-05). A
//! matching one is unpacked into `<servers>/<server>/.partial-<version>-<pid>-<n>`, its
//! program made executable, the folder renamed to `<servers>/<server>/<version>` (when that
//! folder is there already, written by another process, the partial folder is removed and
//! the one there kept), and then `<servers>/<server>/installed.toml` written, through a file
//! renamed into place, naming the version, the digest and the program: from then on the
//! row's downloaded candidate is that version ([`installed`]). The version that was
//! installed before is removed after that. A failure at any step removes the partial file
//! and folder and leaves what was installed before as it was.
//!
//! Of a tar file only regular files and folders are written. An entry of any other kind (a
//! link, a device, a fifo) is refused, and so is one whose path, once `strip` parts are
//! dropped, is absolute or climbs out of the folder: nothing outside the folder is written.
//!
//! Nothing here decides when to download: the Navigator calls [`Downloader::fetch`] only
//! once the user accepted an offer or asked for an update, never for a registered row.

use std::path::{Path, PathBuf};

use crate::registry::{Download, Platform, Registry};

/// The user agent of every request: the app's name, and nothing of the user or the machine.
pub const USER_AGENT: &str = "yaar-review";

/// The most redirects one download follows.
pub const MAX_REDIRECTS: u32 = 5;

/// How long the downloader waits for a connection.
pub const CONNECT_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// How long the downloader waits for each read of an answer.
pub const READ_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

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
    /// The address, or the address a redirect leads to, is not one [`allowed`] or
    /// [`redirect_allowed`] lets the downloader fetch.
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
    /// More bytes arrived than the row's `size`: the size.
    TooLarge(u64),
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
            Self::TooLarge(size) => {
                write!(f, "the download is larger than the {size} bytes pinned")
            }
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

/// Whether `url` is an address the downloader fetches from: `https://` to any host, or
/// `http://` whose host is the address `127.0.0.1` or `[::1]`. Any other scheme, a host
/// name (`localhost` included) over `http`, and a text that is no address, are not.
pub fn allowed(url: &str) -> bool {
    let _ = url;
    todo!()
}

/// Whether a redirect from the address `from` to the address `to` is followed: `to` is
/// [`allowed`], and when `from` is `https://`, `to` is `https://` too.
pub fn redirect_allowed(from: &str, to: &str) -> bool {
    let _ = (from, to);
    todo!()
}

/// The downloader. One is enough for every download of a process.
pub struct Downloader {
    agent: ureq::Agent,
}

impl Downloader {
    /// A downloader with the certificates the system trusts, built as the module doc says:
    /// no proxy, no redirect followed by the library, the user agent [`USER_AGENT`], the
    /// waits [`CONNECT_WITHIN`] and [`READ_WITHIN`]. An error, as a sentence, when no
    /// certificate could be read.
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
