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
//! the one there kept; a folder of that name without the program in it is no install, and is
//! replaced), and then `<servers>/<server>/installed.toml` written, through a file renamed
//! into place, naming the version, the digest and the program: from then on the row's
//! downloaded candidate is that version ([`installed`]). The version that was installed
//! before is removed after that. A failure at any step removes the partial files and
//! folder, and the version folder this fetch made, and leaves what was installed before as
//! it was.
//!
//! Of a tar file only regular files and folders are written. An entry of any other kind (a
//! link, a device, a fifo) is refused, and so is one whose path is absolute or climbs out
//! of the folder, whether `strip` parts are dropped from it first or not: nothing outside
//! the folder is written. A `.` part counts for nothing, and an entry with no part left
//! after the `strip` is passed over, as `tar --strip-components` does. A pax header that
//! only carries notes for the whole archive is read past, being no file. The row's
//! `program` must be a path of plain parts, and be in the unpacked folder when the archive
//! has been read, or the download is refused with [`DownloadError::Unpack`].
//!
//! Nothing here decides when to download: the Navigator calls [`Downloader::fetch`] only
//! once the user accepted an offer or asked for an update, never for a registered row.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ring::digest::{Context, SHA256};
use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};
use url::{Host, Url};

use crate::registry::{Archive, Download, Platform, Registry};

/// The user agent of every request: a fixed name for what the request is, which names
/// neither the user, the machine, the project nor the app's version.
pub const USER_AGENT: &str = "language-server-download";

/// The most redirects one download follows.
pub const MAX_REDIRECTS: u32 = 5;

/// How long the downloader waits for a connection.
pub const CONNECT_WITHIN: Duration = Duration::from_secs(30);

/// How long the downloader waits for each read of an answer.
pub const READ_WITHIN: Duration = Duration::from_secs(60);

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
    let record = read_record(servers, server)?;
    let program = servers
        .join(server)
        .join(&record.version)
        .join(&record.program);
    program.is_file().then_some(Installed {
        version: record.version,
        sha256: record.sha256,
        program,
    })
}

/// What `installed.toml` says, read: its three strings, the version a single plain part and
/// the program a path of plain parts, so that nothing the file names lies outside the
/// server's folder.
struct Record {
    version: String,
    sha256: String,
    program: String,
}

/// `installed.toml` of the server `server` in `servers`: none when it is not there, is not
/// TOML, lacks a string, or names a version or a program that leaves the server's folder.
fn read_record(servers: &Path, server: &str) -> Option<Record> {
    if !plain_part(server) {
        return None;
    }
    let text = fs::read_to_string(servers.join(server).join(INSTALLED_FILE)).ok()?;
    let table: toml::Table = text.parse().ok()?;
    let get = |key: &str| table.get(key)?.as_str().map(str::to_owned);
    let record = Record {
        version: get("version")?,
        sha256: get("sha256")?,
        program: get("program")?,
    };
    (plain_part(&record.version) && plain_path(&record.program).is_some()).then_some(record)
}

/// Whether `name` can be one part of a path below the app's folder: not empty, not `.` or
/// `..`, no separator and no NUL.
fn plain_part(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// `path` as a relative path of plain parts, a `.` part dropped; none when it holds an
/// absolute or climbing part or nothing at all.
fn plain_path(path: &str) -> Option<PathBuf> {
    let mut plain = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(name) => plain.push(name),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!plain.as_os_str().is_empty()).then_some(plain)
}

/// Whether `url` is an address the downloader fetches from: `https://` to any host, or
/// `http://` whose host is the address `127.0.0.1` or `[::1]`. Any other scheme, a host
/// name (`localhost` included) over `http`, and a text that is no address, are not.
pub fn allowed(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    match (url.scheme(), url.host()) {
        ("https", Some(_)) => true,
        ("http", Some(Host::Ipv4(address))) => address == Ipv4Addr::LOCALHOST,
        ("http", Some(Host::Ipv6(address))) => address == Ipv6Addr::LOCALHOST,
        _ => false,
    }
}

/// Whether a redirect from the address `from` to the address `to` is followed: `to` is
/// [`allowed`], and when `from` is `https://`, `to` is `https://` too.
pub fn redirect_allowed(from: &str, to: &str) -> bool {
    let from_https = Url::parse(from).is_ok_and(|from| from.scheme() == "https");
    allowed(to) && (!from_https || Url::parse(to).is_ok_and(|to| to.scheme() == "https"))
}

/// The downloader. One is enough for every download of a process.
pub struct Downloader {
    agent: ureq::Agent,
}

/// The agent every download goes through, as the module doc says, waiting `connect` for a
/// connection and `read` for each read, and trusting `roots`.
fn agent(connect: Duration, read: Duration, roots: RootCerts) -> ureq::Agent {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(roots)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        // `Config::default()` reads the proxy variables of the environment; none is read.
        .proxy(None)
        // The redirects are the downloader's, under its own rule.
        .max_redirects(0)
        .max_redirects_will_error(false)
        // A 404 is an answer the downloader reads, not an error of the library.
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(connect))
        .timeout_per_read(Some(read))
        .build();
    ureq::Agent::new_with_config(config)
}

impl Downloader {
    /// A downloader with the certificates the system trusts, built as the module doc says:
    /// no proxy, no redirect followed by the library, the user agent [`USER_AGENT`], the
    /// waits [`CONNECT_WITHIN`] and [`READ_WITHIN`]. An error, as a sentence, when no
    /// certificate could be read.
    pub fn new() -> Result<Downloader, String> {
        let found = rustls_native_certs::load_native_certs();
        if found.certs.is_empty() {
            let why = found
                .errors
                .first()
                .map_or_else(|| "the system holds none".to_owned(), ToString::to_string);
            return Err(format!(
                "no certificate the system trusts could be read: {why}"
            ));
        }
        let roots = found
            .certs
            .iter()
            .map(|der| Certificate::from_der(der.as_ref()).to_owned());
        Ok(Downloader {
            agent: agent(CONNECT_WITHIN, READ_WITHIN, RootCerts::from(roots)),
        })
    }

    /// Fetches `download` of the server named `server` into `servers`, as the module doc
    /// says, and gives what is installed now.
    pub fn fetch(
        &self,
        server: &str,
        download: &Download,
        servers: &Path,
    ) -> Result<Installed, DownloadError> {
        if !allowed(&download.url) {
            return Err(DownloadError::NotAllowed(download.url.clone()));
        }
        if !plain_part(server) || !plain_part(&download.version) {
            return Err(DownloadError::Folder(format!(
                "{server} {} cannot name a folder",
                download.version
            )));
        }
        let program = plain_path(&download.program).ok_or_else(|| {
            DownloadError::Unpack(format!(
                "the program {:?} is not a path inside the download",
                download.program
            ))
        })?;

        let folder = servers.join(server);
        fs::create_dir_all(&folder).map_err(folder_error)?;
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let id = format!(
            "{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        );
        let partial_file = servers.join(format!(".partial-{server}-{id}"));
        let partial_folder = folder.join(format!(".partial-{}-{id}", download.version));
        let partial_record = folder.join(format!(".partial-installed-{id}"));
        // Whatever way this function ends, what it made and did not rename into place goes.
        let _removed_at_the_end = Removal(vec![
            partial_file.clone(),
            partial_folder.clone(),
            partial_record.clone(),
        ]);

        let found = self.download_into(download, &partial_file)?;
        if found != download.sha256 {
            return Err(DownloadError::Digest {
                expected: download.sha256.clone(),
                found,
            });
        }
        // The digest matched: from here on the bytes are the ones the row pins.
        unpack(&partial_file, download, &program, &partial_folder)?;

        let before = read_record(servers, server).map(|record| record.version);
        let version_folder = folder.join(&download.version);
        let made = place(&partial_folder, &version_folder, &program)?;

        let record = format!(
            "version = {}\nsha256 = {}\nprogram = {}\n",
            toml::Value::from(download.version.as_str()),
            toml::Value::from(download.sha256.as_str()),
            toml::Value::from(download.program.as_str()),
        );
        let written = fs::write(&partial_record, record)
            .and_then(|()| fs::rename(&partial_record, folder.join(INSTALLED_FILE)));
        if let Err(error) = written {
            if made {
                remove(&version_folder);
            }
            return Err(folder_error(error));
        }

        // Switched: the version that was in use before is no longer wanted.
        if let Some(old) = before.filter(|old| *old != download.version && plain_part(old)) {
            remove(&folder.join(old));
        }
        Ok(Installed {
            version: download.version.clone(),
            sha256: download.sha256.clone(),
            program: version_folder.join(&download.program),
        })
    }

    /// Fetches the address of `download` and streams its bytes into the file `partial`,
    /// hashing and counting them as they come. Gives the digest of what arrived.
    fn download_into(&self, download: &Download, partial: &Path) -> Result<String, DownloadError> {
        let response = self.get(&download.url)?;
        if response
            .body()
            .content_length()
            .is_some_and(|length| length > download.size)
        {
            return Err(DownloadError::TooLarge(download.size));
        }
        let mut body = response.into_body().into_reader();
        let mut file = File::create(partial).map_err(folder_error)?;
        let mut context = Context::new(&SHA256);
        let mut counted = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(DownloadError::Network(error.to_string())),
            };
            counted += read as u64;
            if counted > download.size {
                return Err(DownloadError::TooLarge(download.size));
            }
            context.update(&buffer[..read]);
            file.write_all(&buffer[..read]).map_err(folder_error)?;
        }
        file.flush().map_err(folder_error)?;
        Ok(hex(context.finish().as_ref()))
    }

    /// The answer to a `GET` of `url` whose status is 200, redirects followed by hand under
    /// [`redirect_allowed`], at most [`MAX_REDIRECTS`] of them.
    fn get(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>, DownloadError> {
        let mut at = Url::parse(url).map_err(|_| DownloadError::NotAllowed(url.to_owned()))?;
        let mut followed = 0;
        loop {
            let response = self
                .agent
                .get(at.as_str())
                .call()
                .map_err(|error| DownloadError::Network(error.to_string()))?;
            let status = response.status().as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return match status {
                    200 => Ok(response),
                    other => Err(DownloadError::Status(other)),
                };
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    DownloadError::Network(format!(
                        "{at} sent the download elsewhere, to no address"
                    ))
                })?;
            if followed == MAX_REDIRECTS {
                return Err(DownloadError::Network(format!(
                    "more than {MAX_REDIRECTS} redirects, the last from {at}"
                )));
            }
            let next = at.join(location).map_err(|_| {
                DownloadError::Network(format!(
                    "{at} sent the download to {location:?}, no address"
                ))
            })?;
            if !redirect_allowed(at.as_str(), next.as_str()) {
                return Err(DownloadError::NotAllowed(next.to_string()));
            }
            followed += 1;
            at = next;
        }
    }
}

/// Removes, when dropped, each file or folder it holds, whether it is there or not.
struct Removal(Vec<PathBuf>);

impl Drop for Removal {
    fn drop(&mut self) {
        for path in &self.0 {
            remove(path);
        }
    }
}

/// Removes a file, or a folder with what it holds; it is no error that nothing is there.
fn remove(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

fn folder_error(error: std::io::Error) -> DownloadError {
    DownloadError::Folder(error.to_string())
}

fn unpack_error(error: impl std::fmt::Display) -> DownloadError {
    DownloadError::Unpack(error.to_string())
}

/// The digest in lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Unpacks the downloaded file `archive` into the new folder `into`, as the module doc
/// says, and makes `program` executable.
fn unpack(
    archive: &Path,
    download: &Download,
    program: &Path,
    into: &Path,
) -> Result<(), DownloadError> {
    fs::create_dir(into).map_err(folder_error)?;
    let reader = BufReader::new(File::open(archive).map_err(folder_error)?);
    match download.archive {
        Archive::Gz => {
            let target = into.join(program);
            write_file(&mut flate2::read::GzDecoder::new(reader), &target, 0o755)?;
        }
        Archive::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(reader));
            for entry in tar.entries().map_err(unpack_error)? {
                let mut entry = entry.map_err(unpack_error)?;
                let kind = entry.header().entry_type();
                if kind.is_pax_global_extensions() {
                    continue;
                }
                let named = entry.path().map_err(unpack_error)?.into_owned();
                if !kind.is_file() && !kind.is_dir() {
                    return Err(DownloadError::Unpack(format!(
                        "{} is neither a file nor a folder ({kind:?})",
                        named.display()
                    )));
                }
                let Some(relative) = stripped(&named, download.strip)? else {
                    continue;
                };
                let target = into.join(relative);
                if kind.is_dir() {
                    fs::create_dir_all(&target).map_err(unpack_error)?;
                } else {
                    let mode = entry.header().mode().unwrap_or(0o644);
                    let mode = if mode & 0o111 == 0 { 0o644 } else { 0o755 };
                    write_file(&mut entry, &target, mode)?;
                }
            }
        }
    }
    let target = into.join(program);
    if !target.is_file() {
        return Err(DownloadError::Unpack(format!(
            "the download holds no {}",
            program.display()
        )));
    }
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).map_err(folder_error)
}

/// The path of an entry once `strip` parts are dropped, none when no part is left, an error
/// when the path is absolute or climbs.
fn stripped(named: &Path, strip: u32) -> Result<Option<PathBuf>, DownloadError> {
    let mut parts = Vec::new();
    for part in named.components() {
        match part {
            Component::Normal(name) => parts.push(name),
            Component::CurDir => {}
            _ => {
                return Err(DownloadError::Unpack(format!(
                    "{} is absolute or climbs out of the folder",
                    named.display()
                )));
            }
        }
    }
    let left = parts.get(strip as usize..).unwrap_or_default();
    Ok((!left.is_empty()).then(|| left.iter().collect()))
}

/// Writes what `from` gives to the new file `target`, its folders made, with `mode`.
fn write_file(from: &mut dyn Read, target: &Path, mode: u32) -> Result<(), DownloadError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(unpack_error)?;
    }
    let mut file = File::create(target).map_err(unpack_error)?;
    std::io::copy(from, &mut file).map_err(unpack_error)?;
    fs::set_permissions(target, fs::Permissions::from_mode(mode)).map_err(unpack_error)
}

/// Moves the unpacked folder `partial` to `version_folder`. When another process has put a
/// folder with the program in it there already, that one is kept and `partial` is left for
/// the caller to remove; a folder there with no program is no install and is replaced.
/// Whether this call made the folder.
fn place(partial: &Path, version_folder: &Path, program: &Path) -> Result<bool, DownloadError> {
    if version_folder.join(program).is_file() {
        return Ok(false);
    }
    remove(version_folder);
    match fs::rename(partial, version_folder) {
        Ok(()) => Ok(true),
        // Another process won the race to this folder.
        Err(_) if version_folder.join(program).is_file() => Ok(false),
        Err(error) => Err(folder_error(error)),
    }
}

/// Whether `servers` holds the pinned `download` of `server`: `installed.toml` names its
/// version and digest, and its program is there.
fn holds(servers: &Path, server: &str, download: &Download) -> bool {
    installed(servers, server).is_some_and(|held| {
        held.version == download.version
            && held.sha256 == download.sha256
            && held.program
                == servers
                    .join(server)
                    .join(&download.version)
                    .join(&download.program)
    })
}

/// Each server of `registry` with a download for `platform`, and that download, in the
/// order of the rows and of their servers.
fn pinned_servers(
    registry: &Registry,
    platform: Platform,
) -> impl Iterator<Item = (&str, &Download)> {
    registry
        .rows()
        .iter()
        .filter(|row| !row.registered)
        .flat_map(|row| &row.servers)
        .filter_map(move |server| {
            server
                .download(platform)
                .map(|download| (server.name.as_str(), download))
        })
}

/// For each server of each row of `registry` with a download for `platform`: nothing when
/// `servers` holds that version with that digest already, else [`Downloader::fetch`].
/// `told` hears one line for each server, saying which it was, before its download is
/// fetched, so that a failure is named by the last line told. What `cargo xtask
/// language-servers` runs, and nothing else does. The first failure ends it. A downloader
/// is made only for the first server that needs one.
pub fn fill(
    registry: &Registry,
    platform: Platform,
    servers: &Path,
    told: &mut dyn FnMut(&str),
) -> Result<(), DownloadError> {
    let mut downloader = None;
    for (name, download) in pinned_servers(registry, platform) {
        if holds(servers, name, download) {
            told(&format!("{name} {}: there already", download.version));
            continue;
        }
        told(&format!(
            "{name} {}: fetching {}",
            download.version, download.url
        ));
        if downloader.is_none() {
            downloader = Some(Downloader::new().map_err(DownloadError::Network)?);
        }
        if let Some(downloader) = &downloader {
            downloader.fetch(name, download, servers)?;
        }
    }
    Ok(())
}

/// The names of the servers of `registry` with a download for `platform` whose pinned
/// version `servers` does not hold with its digest: what a test that needs the real servers
/// says is missing. Empty when the folder is complete.
pub fn missing(registry: &Registry, platform: Platform, servers: &Path) -> Vec<String> {
    pinned_servers(registry, platform)
        .filter(|(name, download)| !holds(servers, name, download))
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_and_the_loopback_address_over_http_are_fetched_and_nothing_else() {
        for url in [
            "https://example.org/x",
            "https://registry.npmjs.org/@typescript/typescript-darwin-arm64/-/x.tgz",
            "http://127.0.0.1:1/x",
            "http://127.0.0.1/x",
            "http://[::1]:1/x",
        ] {
            assert!(allowed(url), "{url}");
        }
        for url in [
            "http://localhost:1/x",
            "http://example.org/x",
            "http://127.0.0.2:1/x",
            "http://0.0.0.0:1/x",
            "http://[::2]:1/x",
            "ftp://example.org/x",
            "file:///x",
            "https://",
            "https:",
            "not an address",
            "",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn a_redirect_from_https_leads_only_to_https_and_a_loopback_one_may_lead_to_another() {
        assert!(!redirect_allowed(
            "https://a.example/x",
            "http://127.0.0.1:1/x"
        ));
        assert!(redirect_allowed(
            "https://a.example/x",
            "https://b.example/y"
        ));
        assert!(redirect_allowed(
            "http://127.0.0.1:1/x",
            "http://127.0.0.1:2/y"
        ));
        assert!(!redirect_allowed(
            "http://127.0.0.1:1/x",
            "http://example.org/y"
        ));
        // Whatever the address of origin, the next address must be one that is fetched.
        assert!(!redirect_allowed(
            "https://a.example/x",
            "ftp://b.example/y"
        ));
        assert!(redirect_allowed(
            "http://127.0.0.1:1/x",
            "https://b.example/y"
        ));
    }

    #[test]
    fn a_server_that_stops_sending_ends_the_download_when_the_wait_for_a_read_is_over() {
        use std::net::TcpListener;
        use std::sync::mpsc::{RecvTimeoutError, channel};

        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let port = listener.local_addr().expect("its address").port();
        let (release, held) = channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("a connection");
            let mut seen = Vec::new();
            let mut byte = [0_u8; 1];
            while !seen.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => seen.push(byte[0]),
                    _ => return,
                }
            }
            // Ten of a thousand bytes, then silence, the connection kept open until the test
            // has its answer.
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n0123456789");
            let _ = held.recv();
        });

        let downloader = Downloader {
            agent: agent(
                Duration::from_secs(5),
                Duration::from_secs(1),
                RootCerts::Specific(Arc::new(Vec::new())),
            ),
        };
        let folder =
            std::env::temp_dir().join(format!("yaar-download-stall-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let download = Download {
            platform: Platform::LinuxX64,
            version: "1".to_owned(),
            url: format!("http://127.0.0.1:{port}/slow"),
            sha256: "0".repeat(64),
            archive: Archive::Gz,
            strip: 0,
            program: "p".to_owned(),
            size: 1000,
        };
        // The download runs on a thread of its own, so that a missing wait fails this test
        // with a sentence instead of holding it for good.
        let (done, outcome) = channel();
        let target = folder.clone();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _ = done.send(downloader.fetch("slow", &download, &target));
        });
        let outcome = match outcome.recv_timeout(Duration::from_secs(60)) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => {
                panic!("a stalled server holds the download for good")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("the download thread ended"),
        };
        let waited = started.elapsed();
        let _ = release.send(());
        server.join().expect("the stalling server");

        // It ended by the wait, not by some other failure sooner: a slow machine only
        // lengthens what was waited.
        assert!(waited >= Duration::from_secs(1), "ended after {waited:?}");
        assert!(
            matches!(outcome, Err(DownloadError::Network(_))),
            "{outcome:?}"
        );
        let left: Vec<_> = fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        let _ = fs::remove_dir_all(&folder);
        assert!(
            left.iter().all(|name| name == "slow"),
            "no partial file is left: {left:?}"
        );
    }
}
