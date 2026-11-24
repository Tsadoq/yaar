//! Fetches the server a row pins over https (or http to loopback), checks its SHA-256 and unpacks
//! it into `<servers>/<server>/<version>/`.

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

/// A fixed name that names neither the user, the machine, the project nor the app's version.
pub const USER_AGENT: &str = "language-server-download";

pub const MAX_REDIRECTS: u32 = 5;

pub const CONNECT_WITHIN: Duration = Duration::from_secs(30);

pub const READ_WITHIN: Duration = Duration::from_secs(60);

/// Names the version in use: TOML strings `version`, `sha256` and `program` (relative to the
/// version's folder).
pub const INSTALLED_FILE: &str = "installed.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    NotAllowed(String),
    Status(u16),
    Network(String),
    Digest { expected: String, found: String },
    TooLarge(u64),
    Unpack(String),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    pub sha256: String,
    pub program: PathBuf,
}

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

/// `installed.toml` as read: `version` is a single plain part and `program` a path of plain
/// parts, so nothing the file names lies outside the server's folder.
struct Record {
    version: String,
    sha256: String,
    program: String,
}

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

fn plain_part(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

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

/// `http` only to 127.0.0.1 or `[::1]`, never `localhost`, which goes through name resolution.
/// Plain http to loopback is for the tests' own listener; the digest guards what arrives anyway.
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

pub fn redirect_allowed(from: &str, to: &str) -> bool {
    let from_https = Url::parse(from).is_ok_and(|from| from.scheme() == "https");
    allowed(to) && (!from_https || Url::parse(to).is_ok_and(|to| to.scheme() == "https"))
}

pub struct Downloader {
    agent: ureq::Agent,
}

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
        // Redirects are followed by hand, so that none goes from https to http.
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

        if let Some(old) = before.filter(|old| *old != download.version && plain_part(old)) {
            remove(&folder.join(old));
        }
        Ok(Installed {
            version: download.version.clone(),
            sha256: download.sha256.clone(),
            program: version_folder.join(&download.program),
        })
    }

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

struct Removal(Vec<PathBuf>);

impl Drop for Removal {
    fn drop(&mut self) {
        for path in &self.0 {
            remove(path);
        }
    }
}

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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

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

fn write_file(from: &mut dyn Read, target: &Path, mode: u32) -> Result<(), DownloadError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(unpack_error)?;
    }
    let mut file = File::create(target).map_err(unpack_error)?;
    std::io::copy(from, &mut file).map_err(unpack_error)?;
    fs::set_permissions(target, fs::Permissions::from_mode(mode)).map_err(unpack_error)
}

/// A folder already there with the program in it, put by another process, is kept; one without it
/// is replaced. Gives whether this call made the folder.
fn place(partial: &Path, version_folder: &Path, program: &Path) -> Result<bool, DownloadError> {
    if version_folder.join(program).is_file() {
        return Ok(false);
    }
    remove(version_folder);
    match fs::rename(partial, version_folder) {
        Ok(()) => Ok(true),
        Err(_) if version_folder.join(program).is_file() => Ok(false),
        Err(error) => Err(folder_error(error)),
    }
}

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

/// `told` hears a line before each fetch, so a failure is named by the last line.
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
        assert!(!redirect_allowed(
            "https://a.example/x",
            "ftp://b.example/y"
        ));
        assert!(redirect_allowed(
            "http://127.0.0.1:1/x",
            "https://b.example/y"
        ));
    }

    // Test-only certificate and key for 127.0.0.1; the key is public. Valid from 2020 to 2120, not
    // a CA.
    const TEST_CERTIFICATE: &str = "\
-----BEGIN CERTIFICATE-----
MIIBtDCCAVqgAwIBAgIUYpSQHapmF/Opz8hiNEhioquUvyswCgYIKoZIzj0EAwIw
FDESMBAGA1UEAwwJMTI3LjAuMC4xMCAXDTIwMDEwMTAwMDAwMFoYDzIxMjAwMTAx
MDAwMDAwWjAUMRIwEAYDVQQDDAkxMjcuMC4wLjEwWTATBgcqhkjOPQIBBggqhkjO
PQMBBwNCAASykQt2EgWSmXqQ6ilPkk0HIccsGUiIiknCMQxXMg+fLnbpSmMIPZHM
Ez1xkfY5soU97rW7fCf+AHfJmC3lAP9wo4GHMIGEMB0GA1UdDgQWBBTsRr9qh18+
cosA/KXCksshG+wHpDAfBgNVHSMEGDAWgBTsRr9qh18+cosA/KXCksshG+wHpDAP
BgNVHREECDAGhwR/AAABMAwGA1UdEwEB/wQCMAAwDgYDVR0PAQH/BAQDAgeAMBMG
A1UdJQQMMAoGCCsGAQUFBwMBMAoGCCqGSM49BAMCA0gAMEUCIFWX+/BQ/XE+zfny
fjl4mgjDtarMIHrNNA3EVJEKjYNCAiEA2iBx5Vhpnq5pVWMiak6FLn2oMJGilmPy
xf9a/ayOZCY=
-----END CERTIFICATE-----
";
    const TEST_KEY: &str = "\
-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgvJsZTNATq1aw96/J
waBeNJo91rx7YtXd6LYMMNoq1imhRANCAASykQt2EgWSmXqQ6ilPkk0HIccsGUiI
iknCMQxXMg+fLnbpSmMIPZHMEz1xkfY5soU97rW7fCf+AHfJmC3lAP9w
-----END PRIVATE KEY-----
";

    fn redirecting_over_tls(location: String) -> u16 {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};

        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![
                CertificateDer::from_pem_slice(TEST_CERTIFICATE.as_bytes())
                    .expect("the certificate"),
            ],
            PrivateKeyDer::from_pem_slice(TEST_KEY.as_bytes()).expect("the key"),
        )
        .expect("a server configuration");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
        let port = listener.local_addr().expect("its address").port();
        std::thread::spawn(move || {
            let Ok((tcp, _)) = listener.accept() else {
                return;
            };
            let connection = rustls::ServerConnection::new(Arc::new(config)).expect("a connection");
            let mut stream = rustls::StreamOwned::new(connection, tcp);
            let mut seen = Vec::new();
            let mut byte = [0_u8; 1];
            // Reading the request drives the handshake.
            while !seen.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => seen.push(byte[0]),
                    _ => return,
                }
            }
            let _ = stream.write_all(
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n")
                    .as_bytes(),
            );
            stream.conn.send_close_notify();
            let _ = stream.flush();
        });
        port
    }

    fn hearing() -> (u16, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listen");
        let port = listener.local_addr().expect("its address").port();
        let (heard, hearing) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    return;
                };
                let mut seen = Vec::new();
                let mut byte = [0_u8; 1];
                while !seen.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => seen.push(byte[0]),
                        _ => break,
                    }
                }
                let line = String::from_utf8_lossy(&seen);
                let _ = heard.send(line.lines().next().unwrap_or_default().to_owned());
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
        });
        (port, hearing)
    }

    #[test]
    fn a_redirect_from_https_to_the_loopback_address_over_http_is_refused_and_nothing_is_heard_there()
     {
        use rustls::pki_types::pem::PemObject;

        let (plain_port, heard) = hearing();
        let landing = format!("http://127.0.0.1:{plain_port}/landing");
        let tls_port = redirecting_over_tls(landing.clone());
        let certificate =
            rustls::pki_types::CertificateDer::from_pem_slice(TEST_CERTIFICATE.as_bytes())
                .expect("the certificate");
        let downloader = Downloader {
            agent: agent(
                Duration::from_secs(10),
                Duration::from_secs(10),
                RootCerts::Specific(Arc::new(vec![
                    Certificate::from_der(certificate.as_ref()).to_owned(),
                ])),
            ),
        };

        let outcome = downloader.get(&format!("https://127.0.0.1:{tls_port}/server.tgz"));

        assert!(
            allowed(&landing),
            "the landing address is one that is fetched"
        );
        match outcome {
            Err(DownloadError::NotAllowed(address)) => assert_eq!(address, landing),
            Err(other) => panic!("the redirect was not refused by the rule: {other:?}"),
            Ok(_) => panic!("the redirect to {landing} was followed"),
        }
        // Barrier: connections are accepted in arrival order, so a request of the downloader's
        // would be heard before this one.
        let mut own = std::net::TcpStream::connect(("127.0.0.1", plain_port)).expect("connect");
        own.write_all(b"GET /barrier HTTP/1.1\r\n\r\n")
            .expect("send");
        assert_eq!(
            heard
                .recv_timeout(Duration::from_secs(10))
                .expect("the barrier was heard"),
            "GET /barrier HTTP/1.1",
            "something was heard at the address of the redirect before the test's own request"
        );
    }

    #[test]
    fn the_downloader_waits_30_seconds_to_connect_and_60_for_each_read() {
        let downloader = Downloader::new().expect("a downloader");
        let config = downloader.agent.config();

        assert_eq!(CONNECT_WITHIN, Duration::from_secs(30));
        assert_eq!(READ_WITHIN, Duration::from_secs(60));
        assert_eq!(config.timeouts().connect, Some(CONNECT_WITHIN));
        assert_eq!(config.timeout_per_read(), Some(READ_WITHIN));
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
            // Ten of a thousand bytes, then silence, the connection kept open until the test has
            // its answer.
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
        // The download runs on a thread of its own, so that a missing wait fails with a sentence
        // instead of hanging.
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

        // The download ended by the read wait, not by an earlier failure.
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
