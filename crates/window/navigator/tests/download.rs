//! The downloader, through `yaar_navigator::download`: what a request carries, where it may
//! go, what is written, and what is refused. Every address is a `Loopback` of the test's
//! own, so that nothing leaves the machine; the thing downloaded is the stand-in, packed by
//! the test. LSP-05 is the first test.

mod support;

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use support::*;
use yaar_navigator::Platform;
use yaar_navigator::Registry;
use yaar_navigator::download::{
    DownloadError, Downloader, INSTALLED_FILE, Installed, MAX_REDIRECTS, USER_AGENT, allowed, fill,
    installed, missing,
};
use yaar_navigator::registry::{Archive, Download};

/// Where the stand-in lies in the archives these tests pack: one folder above the program,
/// which `strip = 1` drops.
const IN_ARCHIVE: &str = "standin/standin-lsp";

/// The stand-in's bytes.
fn standin_bytes() -> Vec<u8> {
    std::fs::read(standin()).expect("read the stand-in")
}

/// The stand-in packed as a `tar.gz` at [`IN_ARCHIVE`], mode 0755.
fn standin_archive() -> Vec<u8> {
    tar_gz(&[(IN_ARCHIVE, &standin_bytes(), 0o755)])
}

/// What a row of the registry would pin for `bytes` served at `url`: a `tar.gz`, strip 1,
/// the program `standin-lsp`, the size and the digest of `bytes`.
fn pinned(url: String, bytes: &[u8], version: &str) -> Download {
    Download {
        platform: Platform::LinuxX64,
        version: version.to_owned(),
        url,
        sha256: sha256(bytes),
        archive: Archive::TarGz,
        strip: 1,
        program: "standin-lsp".to_owned(),
        size: bytes.len() as u64,
    }
}

/// Every path under `folder`, relative to it, folders and files alike.
fn tree(folder: &Path) -> BTreeSet<PathBuf> {
    fn walk(root: &Path, folder: &Path, found: &mut BTreeSet<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            found.insert(path.strip_prefix(root).expect("below the root").to_owned());
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                walk(root, &path, found);
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(folder, folder, &mut found);
    found
}

/// The entries of `servers` and of `servers/standin` that are a partial file or folder.
fn partials(servers: &Path) -> Vec<PathBuf> {
    [servers.to_path_buf(), servers.join("standin")]
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".partial-"))
        .map(|entry| entry.path())
        .collect()
}

/// The version folders `servers/standin` holds: its entries but `installed.toml`.
fn versions(servers: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(servers.join("standin"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != INSTALLED_FILE)
        .collect();
    found.sort();
    found
}

/// The three strings of `installed.toml` of the server `standin`.
fn installed_file(servers: &Path) -> (String, String, String) {
    let text = std::fs::read_to_string(servers.join("standin").join(INSTALLED_FILE))
        .expect("read installed.toml");
    let table: toml::Table = text.parse().expect("installed.toml is TOML");
    let get = |key: &str| {
        table[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} is a string"))
            .to_owned()
    };
    (get("version"), get("sha256"), get("program"))
}

/// A listener that answers every request with `response`, written as given, and closes. For
/// the answers `Loopback` does not give: no `Location`, a body cut short, a body with no
/// length. Runs until the test process ends.
fn raw_server(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen on the loopback address");
    let port = listener.local_addr().expect("its address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let response = response.clone();
            std::thread::spawn(move || {
                // The request ends with an empty line.
                let mut seen = Vec::new();
                let mut byte = [0_u8; 1];
                while !seen.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => seen.push(byte[0]),
                        _ => return,
                    }
                }
                let _ = stream.write_all(&response);
                let _ = stream.shutdown(std::net::Shutdown::Write);
            });
        }
    });
    format!("http://127.0.0.1:{port}/served")
}

/// A `tar.gz` of `entries` (path, kind, bytes), each path written as given.
fn tar_gz_of(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (path, kind, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_mtime(0);
        header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        if matches!(*kind, tar::EntryType::Symlink | tar::EntryType::Link) {
            header
                .set_link_name("/etc/passwd")
                .expect("name the link's target");
        }
        header.set_cksum();
        builder.append(&header, *bytes).expect("pack the entry");
    }
    builder
        .into_inner()
        .expect("finish the archive")
        .finish()
        .expect("finish the compression")
}

/// A registry of one row a server, each row's server pinned as `(name, version, url,
/// digest, size)` for [`Platform::LinuxX64`].
fn registry_of(servers: &[(&str, &str, String, String, u64)]) -> Registry {
    let mut text = String::new();
    for (name, version, url, digest, size) in servers {
        text.push_str(&format!(
            "[{name}]\n\
             extensions = {{ {name} = \"{name}\" }}\n\
             \n\
             [[{name}.server]]\n\
             name = \"{name}\"\n\
             \n\
             [[{name}.server.download]]\n\
             platform = \"linux-x64\"\n\
             version = \"{version}\"\n\
             url = \"{url}\"\n\
             sha256 = \"{digest}\"\n\
             archive = \"tar.gz\"\n\
             strip = 1\n\
             program = \"standin-lsp\"\n\
             size = {size}\n\n"
        ));
    }
    let (registry, errors) = Registry::read(&text, None);
    assert!(errors.is_empty(), "the registry reads: {errors:?}");
    registry
}

// ---------------------------------------------------------------------------------------
// LSP-05
// ---------------------------------------------------------------------------------------

#[test]
fn a_download_whose_digest_differs_from_its_row_is_refused() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    let downloader = Downloader::new().expect("a downloader");

    // The row pins the digest of other bytes.
    let download = Download {
        sha256: sha256(b"other"),
        ..pinned(loopback.url("standin.tar.gz"), &bytes, "1")
    };
    assert_eq!(
        downloader.fetch("standin", &download, &servers),
        Err(DownloadError::Digest {
            expected: sha256(b"other"),
            found: sha256(&bytes),
        })
    );
    // The bytes did arrive: the refusal is the digest's, not the network's.
    assert_eq!(loopback.heard().len(), 1);
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(installed(&servers, "standin"), None);
    assert!(!servers.join("standin").join(INSTALLED_FILE).exists());
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());

    // With version 1 installed, a version 2 with a wrong digest changes nothing.
    let first = pinned(loopback.url("standin.tar.gz"), &bytes, "1");
    let kept = downloader
        .fetch("standin", &first, &servers)
        .expect("the first version installs");
    let second = Download {
        sha256: sha256(b"other"),
        ..pinned(loopback.url("standin.tar.gz"), &bytes, "2")
    };
    assert_eq!(
        downloader.fetch("standin", &second, &servers),
        Err(DownloadError::Digest {
            expected: sha256(b"other"),
            found: sha256(&bytes),
        })
    );
    assert_eq!(installed(&servers, "standin"), Some(kept.clone()));
    assert_eq!(kept.version, "1");
    assert_eq!(versions(&servers), ["1"]);
    assert_eq!(
        std::fs::read(&kept.program).expect("the first version's program"),
        standin_bytes()
    );
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
}

// ---------------------------------------------------------------------------------------
// What is installed
// ---------------------------------------------------------------------------------------

#[test]
fn a_tar_gz_installs_the_program_executable_and_installed_toml_names_it() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = tar_gz(&[
        (IN_ARCHIVE, &standin_bytes(), 0o755),
        ("standin/lib/data.txt", b"data", 0o600),
    ]);
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    let download = pinned(loopback.url("standin.tar.gz"), &bytes, "1");

    let done = Downloader::new()
        .expect("a downloader")
        .fetch("standin", &download, &servers)
        .expect("the download installs");

    let program = servers.join("standin/1/standin-lsp");
    assert_eq!(
        done,
        Installed {
            version: "1".to_owned(),
            sha256: sha256(&bytes),
            program: program.clone(),
        }
    );
    assert_eq!(std::fs::read(&program).unwrap(), standin_bytes());
    let mode = std::os::unix::fs::PermissionsExt::mode(&program.metadata().unwrap().permissions());
    assert_eq!(mode & 0o111, 0o111, "the program is executable");
    // The path above `strip` is gone and the rest kept.
    assert_eq!(
        std::fs::read(servers.join("standin/1/lib/data.txt")).unwrap(),
        b"data"
    );
    assert!(!servers.join("standin/1/standin").exists());
    assert_eq!(
        installed_file(&servers),
        ("1".to_owned(), sha256(&bytes), "standin-lsp".to_owned())
    );
    assert_eq!(installed(&servers, "standin"), Some(done));
    assert_eq!(installed(&servers, "another"), None);
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
    // The stand-in the download holds runs: the unpacked program is the one served.
    let output = Command::new(&program).arg("--version").output().unwrap();
    assert!(output.status.success());
}

#[test]
fn a_gz_installs_its_one_file_under_the_programs_name_executable() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = gz(&standin_bytes());
    let loopback = Loopback::serve(vec![("standin.gz".to_owned(), bytes.clone())]);
    let download = Download {
        archive: Archive::Gz,
        strip: 0,
        program: "rust-analyzer".to_owned(),
        ..pinned(loopback.url("standin.gz"), &bytes, "2026-11-09")
    };

    let done = Downloader::new()
        .expect("a downloader")
        .fetch("standin", &download, &servers)
        .expect("the download installs");

    let program = servers.join("standin/2026-11-09/rust-analyzer");
    assert_eq!(done.program, program);
    assert_eq!(std::fs::read(&program).unwrap(), standin_bytes());
    let mode = std::os::unix::fs::PermissionsExt::mode(&program.metadata().unwrap().permissions());
    assert_eq!(mode & 0o111, 0o111, "the program is executable");
    assert_eq!(
        installed_file(&servers),
        (
            "2026-11-09".to_owned(),
            sha256(&bytes),
            "rust-analyzer".to_owned()
        )
    );
    assert_eq!(installed(&servers, "standin"), Some(done));
}

#[test]
fn a_second_version_installs_beside_the_first_and_the_first_is_gone() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let one = standin_archive();
    let two = tar_gz(&[(IN_ARCHIVE, b"#!/bin/sh\nexit 0\n", 0o755)]);
    let loopback = Loopback::serve(vec![
        ("one.tar.gz".to_owned(), one.clone()),
        ("two.tar.gz".to_owned(), two.clone()),
    ]);
    let downloader = Downloader::new().expect("a downloader");

    let first = downloader
        .fetch(
            "standin",
            &pinned(loopback.url("one.tar.gz"), &one, "1"),
            &servers,
        )
        .unwrap();
    assert_eq!(versions(&servers), ["1"]);
    let second = downloader
        .fetch(
            "standin",
            &pinned(loopback.url("two.tar.gz"), &two, "2"),
            &servers,
        )
        .unwrap();

    assert_eq!(first.program, servers.join("standin/1/standin-lsp"));
    assert_eq!(second.program, servers.join("standin/2/standin-lsp"));
    assert_eq!(installed(&servers, "standin"), Some(second));
    assert_eq!(
        installed_file(&servers),
        ("2".to_owned(), sha256(&two), "standin-lsp".to_owned())
    );
    assert_eq!(
        versions(&servers),
        ["2"],
        "the first version's folder is gone"
    );
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
}

#[test]
fn installed_toml_is_replaced_by_a_rename_not_written_in_place() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let one = standin_archive();
    let two = tar_gz(&[(IN_ARCHIVE, b"#!/bin/sh\nexit 0\n", 0o755)]);
    let loopback = Loopback::serve(vec![
        ("one.tar.gz".to_owned(), one.clone()),
        ("two.tar.gz".to_owned(), two.clone()),
    ]);
    let downloader = Downloader::new().expect("a downloader");
    downloader
        .fetch(
            "standin",
            &pinned(loopback.url("one.tar.gz"), &one, "1"),
            &servers,
        )
        .unwrap();

    // A second name for the file the first install wrote: a write in place would reach it,
    // a rename of a new file over `installed.toml` leaves it as it was.
    let file = servers.join("standin").join(INSTALLED_FILE);
    let held = scratch.root.join("held-installed.toml");
    std::fs::hard_link(&file, &held).unwrap();
    let before = std::fs::read(&held).unwrap();

    downloader
        .fetch(
            "standin",
            &pinned(loopback.url("two.tar.gz"), &two, "2"),
            &servers,
        )
        .unwrap();

    assert_eq!(std::fs::read(&held).unwrap(), before);
    assert_ne!(std::fs::read(&file).unwrap(), before);
    assert_eq!(installed_file(&servers).0, "2");
}

#[test]
fn a_version_folder_another_process_wrote_is_kept_and_a_damaged_one_is_replaced() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    let downloader = Downloader::new().expect("a downloader");

    // Another process unpacked version 1 first; its program is what stays.
    let there = servers.join("standin/1/standin-lsp");
    std::fs::create_dir_all(there.parent().unwrap()).unwrap();
    std::fs::write(&there, b"written by another process").unwrap();
    let done = downloader
        .fetch(
            "standin",
            &pinned(loopback.url("standin.tar.gz"), &bytes, "1"),
            &servers,
        )
        .unwrap();
    assert_eq!(done.program, there);
    assert_eq!(
        std::fs::read(&there).unwrap(),
        b"written by another process"
    );
    assert_eq!(installed(&servers, "standin"), Some(done));
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());

    // A folder of version 2 with no program in it is no install: the download replaces it.
    std::fs::create_dir_all(servers.join("standin/2/lib")).unwrap();
    let done = downloader
        .fetch(
            "standin",
            &pinned(loopback.url("standin.tar.gz"), &bytes, "2"),
            &servers,
        )
        .unwrap();
    assert_eq!(std::fs::read(&done.program).unwrap(), standin_bytes());
    assert!(!servers.join("standin/2/lib").exists());
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
}

#[test]
fn installed_reads_installed_toml_as_the_module_documents_it() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let write = |server: &str, text: &str| {
        std::fs::create_dir_all(servers.join(server)).unwrap();
        std::fs::write(servers.join(server).join(INSTALLED_FILE), text).unwrap();
    };
    let program = |server: &str, path: &str| {
        let program = servers.join(server).join(path);
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, b"program").unwrap();
        program
    };

    // The file as it is written in the folder of real servers: a program under a folder.
    let tsc = program("tsgo", "7.0.2/lib/tsc");
    write(
        "tsgo",
        "version = \"7.0.2\"\nsha256 = \"902e\"\nprogram = \"lib/tsc\"\n",
    );
    assert_eq!(
        installed(&servers, "tsgo"),
        Some(Installed {
            version: "7.0.2".to_owned(),
            sha256: "902e".to_owned(),
            program: tsc,
        })
    );

    // Nothing there, or a file that is no record: none.
    assert_eq!(installed(&servers, "nothing"), None);
    for (server, text) in [
        ("not-toml", "version = "),
        ("no-sha", "version = \"1\"\nprogram = \"p\"\n"),
        ("a-number", "version = 1\nsha256 = \"x\"\nprogram = \"p\"\n"),
        ("empty", ""),
    ] {
        program(server, "1/p");
        write(server, text);
        assert_eq!(installed(&servers, server), None, "{server}");
    }
    // A record whose program is not in the folder: the server is not there.
    write("gone", "version = \"1\"\nsha256 = \"x\"\nprogram = \"p\"\n");
    assert_eq!(installed(&servers, "gone"), None);
    // A record that names a place outside the server's folder: never followed.
    std::fs::write(scratch.root.join("outside"), b"x").unwrap();
    for (version, program_name) in [
        ("../..", "outside"),
        ("1", "../../../outside"),
        ("1", "/etc/hosts"),
        ("", "p"),
        ("1", ""),
    ] {
        program("escape", "1/p");
        write(
            "escape",
            &format!("version = \"{version}\"\nsha256 = \"x\"\nprogram = \"{program_name}\"\n"),
        );
        assert_eq!(
            installed(&servers, "escape"),
            None,
            "{version} {program_name}"
        );
    }
    // A server named by a path is no server.
    assert_eq!(installed(&servers, "../servers/tsgo"), None);
    assert_eq!(installed(&servers, ""), None);
}

// ---------------------------------------------------------------------------------------
// What a request carries, and where it may go
// ---------------------------------------------------------------------------------------

#[test]
fn the_request_carries_exactly_host_user_agent_and_accept() {
    let scratch = Scratch::new();
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    let download = pinned(loopback.url("standin.tar.gz"), &bytes, "1");
    Downloader::new()
        .expect("a downloader")
        .fetch("standin", &download, &scratch.root.join("servers"))
        .expect("the download installs");

    let heard = loopback.heard();
    assert_eq!(heard.len(), 1, "one GET");
    assert_eq!(heard[0].line, "GET /standin.tar.gz");
    let mut names: Vec<&str> = heard[0].headers.iter().map(|(n, _)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["accept", "host", "user-agent"]);
    let value = |name: &str| {
        heard[0]
            .headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .unwrap()
    };
    assert_eq!(USER_AGENT, "language-server-download");
    assert_eq!(value("user-agent"), "language-server-download");
    assert_eq!(value("accept"), "*/*");
    assert!(value("host").starts_with("127.0.0.1:"));
}

#[test]
fn an_address_the_downloader_does_not_fetch_is_refused_before_anything_is_sent() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    let downloader = Downloader::new().expect("a downloader");
    let port = loopback
        .url("")
        .rsplit(':')
        .next()
        .unwrap()
        .trim_end_matches('/')
        .to_owned();

    for url in [
        format!("http://localhost:{port}/standin.tar.gz"),
        "http://example.org/standin.tar.gz".to_owned(),
        "ftp://example.org/standin.tar.gz".to_owned(),
        "file:///etc/passwd".to_owned(),
        "not an address".to_owned(),
    ] {
        assert!(!allowed(&url), "{url} is not allowed");
        let download = pinned(url.clone(), &bytes, "1");
        assert_eq!(
            downloader.fetch("standin", &download, &servers),
            Err(DownloadError::NotAllowed(url)),
        );
    }
    // The barrier: a fetch of an address that is allowed, through the same path, ends before
    // the listener's list is read, and its request is the only one on it.
    let download = pinned(loopback.url("standin.tar.gz"), &bytes, "1");
    downloader
        .fetch("standin", &download, &servers)
        .expect("the allowed address installs");
    assert_eq!(
        loopback
            .heard()
            .iter()
            .map(|h| h.line.as_str())
            .collect::<Vec<_>>(),
        ["GET /standin.tar.gz"]
    );
}

#[test]
fn a_relative_location_is_joined_to_the_address_that_answered_and_followed() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("b".to_owned(), bytes.clone())]);
    loopback.redirect("a", "/b");

    let done = Downloader::new()
        .expect("a downloader")
        .fetch("standin", &pinned(loopback.url("a"), &bytes, "1"), &servers)
        .expect("the redirect is followed");

    assert_eq!(done.version, "1");
    let lines: Vec<String> = loopback.heard().into_iter().map(|h| h.line).collect();
    assert_eq!(lines, ["GET /a", "GET /b"]);
}

#[test]
fn an_absolute_location_to_another_loopback_listener_is_followed() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let publisher = Loopback::serve(vec![("file".to_owned(), bytes.clone())]);
    let front = Loopback::serve(Vec::new());
    front.redirect("go", &publisher.url("file"));

    Downloader::new()
        .expect("a downloader")
        .fetch("standin", &pinned(front.url("go"), &bytes, "1"), &servers)
        .expect("the redirect is followed");

    assert_eq!(front.heard().len(), 1);
    assert_eq!(publisher.heard().len(), 1);
    assert_eq!(publisher.heard()[0].line, "GET /file");
}

#[test]
fn a_redirect_to_an_address_that_is_not_fetched_is_refused_naming_it_and_nothing_is_sent_there() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    loopback.redirect("c", "http://example.invalid/c");

    let downloader = Downloader::new().expect("a downloader");
    assert_eq!(
        downloader.fetch("standin", &pinned(loopback.url("c"), &bytes, "1"), &servers),
        Err(DownloadError::NotAllowed(
            "http://example.invalid/c".to_owned()
        ))
    );
    let lines: Vec<String> = loopback.heard().into_iter().map(|h| h.line).collect();
    assert_eq!(lines, ["GET /c"]);
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
}

#[test]
fn at_most_five_redirects_are_followed() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    assert_eq!(MAX_REDIRECTS, 5);
    let loopback = Loopback::serve(vec![("file".to_owned(), bytes.clone())]);
    let downloader = Downloader::new().expect("a downloader");

    // Six redirects: `r1` to `r6`, and `r6` to the file.
    for n in 1..=5 {
        loopback.redirect(&format!("r{n}"), &format!("/r{}", n + 1));
    }
    loopback.redirect("r6", "/file");
    let error = downloader
        .fetch(
            "standin",
            &pinned(loopback.url("r1"), &bytes, "1"),
            &servers,
        )
        .expect_err("six redirects are one too many");
    assert!(matches!(error, DownloadError::Network(_)), "{error:?}");
    let lines: Vec<String> = loopback.heard().into_iter().map(|h| h.line).collect();
    assert_eq!(
        lines,
        [
            "GET /r1", "GET /r2", "GET /r3", "GET /r4", "GET /r5", "GET /r6"
        ]
    );
    assert_eq!(versions(&servers), Vec::<String>::new());

    // Five: `r1` to `r5`, and `r5` to the file.
    loopback.redirect("r5", "/file");
    downloader
        .fetch(
            "standin",
            &pinned(loopback.url("r1"), &bytes, "1"),
            &servers,
        )
        .expect("five redirects are followed");
    assert_eq!(versions(&servers), ["1"]);
}

#[test]
fn a_redirect_with_no_location_or_with_one_that_is_no_address_is_a_network_failure() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let downloader = Downloader::new().expect("a downloader");

    let none = raw_server(b"HTTP/1.1 301 Moved Permanently\r\nContent-Length: 0\r\n\r\n".to_vec());
    let error = downloader
        .fetch("standin", &pinned(none, &bytes, "1"), &servers)
        .expect_err("a redirect that leads nowhere");
    assert!(matches!(error, DownloadError::Network(_)), "{error:?}");

    let loopback = Loopback::serve(Vec::new());
    loopback.redirect("bad", "http://");
    let error = downloader
        .fetch(
            "standin",
            &pinned(loopback.url("bad"), &bytes, "1"),
            &servers,
        )
        .expect_err("a location that is no address");
    assert!(matches!(error, DownloadError::Network(_)), "{error:?}");
    assert_eq!(versions(&servers), Vec::<String>::new());
}

#[test]
fn a_proxy_named_in_the_environment_is_not_used() {
    let this_test = "a_proxy_named_in_the_environment_is_not_used";
    if std::env::var_os("YAAR_PROXY_CHILD").is_some() {
        // The child process, started with every proxy variable set to the would-be proxy.
        let scratch = Scratch::new();
        let bytes = standin_archive();
        let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
        let download = pinned(loopback.url("standin.tar.gz"), &bytes, "1");
        Downloader::new()
            .expect("a downloader")
            .fetch("standin", &download, &scratch.root.join("servers"))
            .expect("the download goes to its address, not to the proxy");
        assert_eq!(loopback.heard().len(), 1, "the address heard the request");
        return;
    }

    // The would-be proxy serves nothing: an answer through it would be a 404.
    let proxy = Loopback::serve(Vec::new());
    let address = proxy.url("").trim_end_matches('/').to_owned();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", this_test, "--nocapture", "--test-threads=1"])
        .env("YAAR_PROXY_CHILD", "1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        child.env(name, &address);
    }
    let output = child
        .output()
        .expect("run the test in a process of its own");
    let said = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{said}");
    assert!(said.contains("1 passed"), "the child test ran: {said}");
    // The child's download finished before it ended, so a request to the proxy would be on
    // its list by now.
    assert_eq!(proxy.heard(), []);
}

#[test]
fn an_https_address_is_tried_and_a_refused_connection_is_a_network_failure() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    // A port nothing listens on: bound, then let go.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("https://127.0.0.1:{port}/standin.tar.gz");
    assert!(allowed(&url));

    let error = Downloader::new()
        .expect("a downloader")
        .fetch("standin", &pinned(url, &bytes, "1"), &servers)
        .expect_err("nothing answers");
    assert!(matches!(error, DownloadError::Network(_)), "{error:?}");
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
}

// ---------------------------------------------------------------------------------------
// What stops a download
// ---------------------------------------------------------------------------------------

#[test]
fn more_bytes_than_the_rows_size_stop_the_download_and_leave_nothing() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(vec![("standin.tar.gz".to_owned(), bytes.clone())]);
    let size = bytes.len() as u64 - 10;
    let download = Download {
        size,
        ..pinned(loopback.url("standin.tar.gz"), &bytes, "1")
    };

    assert_eq!(
        Downloader::new()
            .expect("a downloader")
            .fetch("standin", &download, &servers),
        Err(DownloadError::TooLarge(size))
    );
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(installed(&servers, "standin"), None);
    assert!(!servers.join("standin").join(INSTALLED_FILE).exists());
}

#[test]
fn bytes_that_stream_with_no_length_are_counted_and_stop_at_the_size() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let body = vec![7_u8; 4096];
    // Chunked: no length is announced, so only the count of what streams in can stop it.
    let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..4 {
        response.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
        response.extend_from_slice(&body);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    let url = raw_server(response);
    let download = Download {
        size: 10_000,
        ..pinned(url, &[0; 16384], "1")
    };

    assert_eq!(
        Downloader::new()
            .expect("a downloader")
            .fetch("standin", &download, &servers),
        Err(DownloadError::TooLarge(10_000))
    );
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
    assert_eq!(versions(&servers), Vec::<String>::new());
}

#[test]
fn a_body_cut_short_is_a_network_failure_and_leaves_no_partial_file() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let url = raw_server(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nonly ten b".to_vec());

    let error = Downloader::new()
        .expect("a downloader")
        .fetch("standin", &pinned(url, &bytes, "1"), &servers)
        .expect_err("the answer ends before its length");
    assert!(matches!(error, DownloadError::Network(_)), "{error:?}");
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(installed(&servers, "standin"), None);
}

#[test]
fn a_status_that_is_not_ok_is_its_own_error_and_installs_nothing() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = standin_archive();
    let loopback = Loopback::serve(Vec::new());

    assert_eq!(
        Downloader::new().expect("a downloader").fetch(
            "standin",
            &pinned(loopback.url("missing.tar.gz"), &bytes, "1"),
            &servers
        ),
        Err(DownloadError::Status(404))
    );
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(installed(&servers, "standin"), None);
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
}

// ---------------------------------------------------------------------------------------
// What is refused in an archive
// ---------------------------------------------------------------------------------------

/// Fetches an archive of `entries` whose digest the row pins, so that it reaches the unpacking,
/// and checks that it was refused with [`DownloadError::Unpack`], that nothing was written
/// anywhere in the scratch but the server's own folder, and that nothing is installed.
fn assert_unpack_refuses(
    scratch: &Scratch,
    strip: u32,
    entries: &[(&str, tar::EntryType, &[u8])],
) -> String {
    // A folder of its own for each call, so that one test can try several archives.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let servers = scratch.root.join(format!("refused-{n}"));
    let bytes = tar_gz_of(entries);
    let loopback = Loopback::serve(vec![("bad.tar.gz".to_owned(), bytes.clone())]);
    let download = Download {
        strip,
        ..pinned(loopback.url("bad.tar.gz"), &bytes, "1")
    };
    let before = tree(&scratch.root);

    let error = Downloader::new()
        .expect("a downloader")
        .fetch("standin", &download, &servers)
        .expect_err("the archive is refused");
    let DownloadError::Unpack(reason) = error else {
        panic!("not an unpacking failure: {error:?}");
    };

    let created: BTreeSet<_> = tree(&scratch.root).difference(&before).cloned().collect();
    let own = PathBuf::from(format!("refused-{n}/standin"));
    assert!(
        created
            .iter()
            .all(|path| *path == own || *path == Path::new(&format!("refused-{n}"))),
        "nothing is written outside the server's folder: {created:?}"
    );
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
    assert_eq!(versions(&servers), Vec::<String>::new());
    assert_eq!(installed(&servers, "standin"), None);
    assert!(!Path::new("/yaar-escape-test").exists());
    reason
}

/// One entry of an archive: its path as written, its kind, its bytes.
fn entry<'a>(
    path: &'a str,
    kind: tar::EntryType,
    bytes: &'a [u8],
) -> (&'a str, tar::EntryType, &'a [u8]) {
    (path, kind, bytes)
}

#[test]
fn an_entry_that_climbs_out_of_the_folder_is_refused() {
    use tar::EntryType::Regular;
    let scratch = Scratch::new();
    // Before the strip and after it: dropping a part must not make a climb safe or unsafe.
    assert_unpack_refuses(&scratch, 0, &[entry("../escape", Regular, b"x")]);
    assert_unpack_refuses(&scratch, 1, &[entry("standin/../../escape", Regular, b"x")]);
    assert_unpack_refuses(
        &scratch,
        1,
        &[
            entry(IN_ARCHIVE, Regular, b"ok"),
            entry("standin/a/../../../b", Regular, b"x"),
        ],
    );
}

#[test]
fn an_absolute_entry_is_refused() {
    use tar::EntryType::Regular;
    let scratch = Scratch::new();
    assert_unpack_refuses(&scratch, 0, &[entry("/yaar-escape-test", Regular, b"x")]);
    assert_unpack_refuses(
        &scratch,
        1,
        &[
            entry(IN_ARCHIVE, Regular, b"ok"),
            entry("/yaar-escape-test", Regular, b"x"),
        ],
    );
}

#[test]
fn a_link_and_a_fifo_are_refused() {
    use tar::EntryType::{Directory, Fifo, Link, Regular, Symlink};
    let scratch = Scratch::new();
    let program = entry(IN_ARCHIVE, Regular, b"ok");
    assert_unpack_refuses(&scratch, 1, &[program, entry("standin/link", Symlink, b"")]);
    assert_unpack_refuses(&scratch, 1, &[program, entry("standin/hard", Link, b"")]);
    assert_unpack_refuses(&scratch, 1, &[program, entry("standin/pipe", Fifo, b"")]);
    assert_unpack_refuses(
        &scratch,
        1,
        &[
            program,
            entry("standin/dir/", Directory, b""),
            entry("standin/dir/pipe", Fifo, b""),
        ],
    );
}

#[test]
fn an_archive_without_the_rows_program_is_refused() {
    use tar::EntryType::Regular;
    let scratch = Scratch::new();
    let reason = assert_unpack_refuses(&scratch, 1, &[entry("standin/other", Regular, b"x")]);
    assert!(
        reason.contains("standin-lsp"),
        "the reason names the program: {reason}"
    );
}

#[test]
fn a_program_that_names_a_path_outside_the_folder_is_refused() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let bytes = gz(b"x");
    let loopback = Loopback::serve(vec![("x.gz".to_owned(), bytes.clone())]);
    let downloader = Downloader::new().expect("a downloader");
    let before = tree(&scratch.root);

    for program in ["../escape", "/yaar-escape-test", ""] {
        let download = Download {
            archive: Archive::Gz,
            strip: 0,
            program: program.to_owned(),
            ..pinned(loopback.url("x.gz"), &bytes, "1")
        };
        let error = downloader
            .fetch("standin", &download, &servers)
            .expect_err("the program's name is refused");
        assert!(
            matches!(error, DownloadError::Unpack(_)),
            "{program}: {error:?}"
        );
    }
    let created: BTreeSet<_> = tree(&scratch.root).difference(&before).cloned().collect();
    assert!(
        created
            .iter()
            .all(|path| path == Path::new("servers/standin")),
        "{created:?}"
    );
    assert_eq!(partials(&servers), Vec::<PathBuf>::new());
    assert!(!Path::new("/yaar-escape-test").exists());
}

// ---------------------------------------------------------------------------------------
// fill and missing
// ---------------------------------------------------------------------------------------

#[test]
fn fill_fetches_what_the_folder_lacks_and_a_second_fill_fetches_nothing() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let one = standin_archive();
    let two = tar_gz(&[(IN_ARCHIVE, b"#!/bin/sh\nexit 0\n", 0o755)]);
    let loopback = Loopback::serve(vec![
        ("one.tar.gz".to_owned(), one.clone()),
        ("two.tar.gz".to_owned(), two.clone()),
    ]);
    let registry = registry_of(&[
        (
            "standin",
            "1",
            loopback.url("one.tar.gz"),
            sha256(&one),
            one.len() as u64,
        ),
        (
            "second",
            "7",
            loopback.url("two.tar.gz"),
            sha256(&two),
            two.len() as u64,
        ),
    ]);
    assert_eq!(
        missing(&registry, Platform::LinuxX64, &servers),
        ["standin", "second"]
    );

    let mut told = Vec::new();
    fill(&registry, Platform::LinuxX64, &servers, &mut |line| {
        told.push(line.to_owned())
    })
    .expect("the folder fills");
    let fetched: Vec<String> = loopback.heard().into_iter().map(|h| h.line).collect();
    assert_eq!(fetched, ["GET /one.tar.gz", "GET /two.tar.gz"]);
    assert_eq!(told.len(), 2, "one line for each server: {told:?}");
    assert!(
        told[0].contains("standin") && told[0].contains('1'),
        "{told:?}"
    );
    assert!(
        told[1].contains("second") && told[1].contains('7'),
        "{told:?}"
    );
    assert_eq!(
        installed(&servers, "standin").map(|i| i.version),
        Some("1".to_owned())
    );
    assert_eq!(
        installed(&servers, "second").map(|i| i.version),
        Some("7".to_owned())
    );
    assert_eq!(
        missing(&registry, Platform::LinuxX64, &servers),
        Vec::<String>::new()
    );

    // The folder holds the pinned versions: nothing is fetched, and each line says so.
    let mut again = Vec::new();
    fill(&registry, Platform::LinuxX64, &servers, &mut |line| {
        again.push(line.to_owned())
    })
    .expect("a filled folder is found whole");
    assert_eq!(loopback.heard().len(), 2, "no request was added");
    assert_eq!(again.len(), 2);
    assert_ne!(again, told, "the lines say what each server was");
    assert!(
        again.iter().all(|line| line.contains("already")),
        "{again:?}"
    );

    // No other platform's row is touched.
    let mut none = Vec::new();
    fill(&registry, Platform::DarwinArm64, &servers, &mut |line| {
        none.push(line.to_owned())
    })
    .expect("a platform with no download needs nothing");
    assert!(none.is_empty());
    assert_eq!(loopback.heard().len(), 2);
}

#[test]
fn fill_stops_at_the_first_download_that_fails_and_says_which_server_it_was_on() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let one = standin_archive();
    let loopback = Loopback::serve(vec![("one.tar.gz".to_owned(), one.clone())]);
    let registry = registry_of(&[
        (
            "standin",
            "1",
            loopback.url("gone.tar.gz"),
            sha256(&one),
            one.len() as u64,
        ),
        (
            "second",
            "7",
            loopback.url("one.tar.gz"),
            sha256(&one),
            one.len() as u64,
        ),
    ]);

    let mut told = Vec::new();
    let error = fill(&registry, Platform::LinuxX64, &servers, &mut |line| {
        told.push(line.to_owned())
    })
    .expect_err("the first download is a 404");
    assert_eq!(error, DownloadError::Status(404));
    assert_eq!(told.len(), 1, "the second server was not tried: {told:?}");
    assert!(told[0].contains("standin"), "{told:?}");
    assert_eq!(loopback.heard().len(), 1);
}

#[test]
fn missing_names_a_server_whose_installed_toml_names_another_version_or_digest() {
    let scratch = Scratch::new();
    let servers = scratch.root.join("servers");
    let one = standin_archive();
    let loopback = Loopback::serve(vec![("one.tar.gz".to_owned(), one.clone())]);
    let size = one.len() as u64;
    let version_one = registry_of(&[(
        "standin",
        "1",
        loopback.url("one.tar.gz"),
        sha256(&one),
        size,
    )]);
    fill(&version_one, Platform::LinuxX64, &servers, &mut |_| {}).expect("the folder fills");
    assert_eq!(
        missing(&version_one, Platform::LinuxX64, &servers),
        Vec::<String>::new()
    );

    // The row now names version 2, then version 1 with another digest.
    let version_two = registry_of(&[(
        "standin",
        "2",
        loopback.url("one.tar.gz"),
        sha256(&one),
        size,
    )]);
    assert_eq!(
        missing(&version_two, Platform::LinuxX64, &servers),
        ["standin"]
    );
    let other_digest = registry_of(&[(
        "standin",
        "1",
        loopback.url("one.tar.gz"),
        sha256(b"other"),
        size,
    )]);
    assert_eq!(
        missing(&other_digest, Platform::LinuxX64, &servers),
        ["standin"]
    );

    // A server whose program is gone from the folder is missing too.
    std::fs::remove_file(servers.join("standin/1/standin-lsp")).unwrap();
    assert_eq!(
        missing(&version_one, Platform::LinuxX64, &servers),
        ["standin"]
    );
    // A row with no download for the platform is nobody's to fill.
    assert_eq!(
        missing(&version_one, Platform::DarwinArm64, &servers),
        Vec::<String>::new()
    );
}

#[test]
fn the_built_in_table_names_a_download_for_each_platform_the_downloader_knows() {
    // What `cargo xtask language-servers` fills: three servers, each with all four.
    let (registry, errors) = Registry::read(yaar_navigator::BUILTIN, None);
    assert!(errors.is_empty(), "{errors:?}");
    let scratch = Scratch::new();
    for platform in Platform::ALL {
        let names: Vec<String> = registry
            .rows()
            .iter()
            .flat_map(|row| &row.servers)
            .filter(|server| server.download(platform).is_some())
            .map(|server| server.name.clone())
            .collect();
        assert_eq!(
            names,
            ["tsgo", "ty", "rust-analyzer"],
            "{}",
            platform.name()
        );
        // An empty folder lacks each, in the table's order.
        assert_eq!(
            missing(&registry, platform, &scratch.root.join("servers")),
            ["tsgo", "ty", "rust-analyzer"]
        );
    }
}
