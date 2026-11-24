//! `cargo xtask language-servers <folder>`: fills `<folder>` with the language servers the
//! Navigator's table pins for this machine, through the app's own downloader. A row already held at
//! its pinned version and digest is not fetched again.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use yaar_navigator::download::{self, DownloadError};
use yaar_navigator::{BUILTIN, Platform, Registry};

pub fn fill(folder: &Path) -> ExitCode {
    let Some(platform) = Platform::current() else {
        eprintln!(
            "language-servers: the built-in table downloads no language server for this machine ({} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        return ExitCode::FAILURE;
    };
    let (registry, errors) = Registry::read(BUILTIN, None);
    if !errors.is_empty() {
        for error in errors {
            eprintln!("language-servers: the built-in table: {error:?}");
        }
        return ExitCode::FAILURE;
    }
    let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
    match run(&registry, platform, &folder, &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("language-servers: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    registry: &Registry,
    platform: Platform,
    folder: &Path,
    out: &mut dyn Write,
) -> Result<(), DownloadError> {
    download::fill(registry, platform, folder, &mut |line| {
        // A line that cannot be written is the terminal's loss, not the download's.
        let _ = writeln!(out, "language-servers: {line}");
        let _ = out.flush();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// `#!/bin/sh` and `exit 0`, one gzip file: what the listener serves.
    const SERVED: [u8; 37] = [
        0x1f, 0x8b, 0x8, 0x0, 0x0, 0x0, 0x0, 0x0, 0x2, 0xff, 0x53, 0x56, 0xd4, 0x4f, 0xca, 0xcc,
        0xd3, 0x2f, 0xce, 0xe0, 0x4a, 0xad, 0xc8, 0x2c, 0x51, 0x30, 0xe0, 0x2, 0x0, 0xe2, 0x86,
        0x45, 0xc3, 0x11, 0x0, 0x0, 0x0,
    ];

    const DIGEST: &str = "79990711ecb64d8f14884c347c625155596f887366f92ebf45a37c89b346fcc6";

    fn serve() -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let port = listener.local_addr().expect("its address").port();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&heard);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut seen = Vec::new();
                let mut byte = [0_u8; 1];
                while !seen.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => seen.push(byte[0]),
                        _ => break,
                    }
                }
                let line = String::from_utf8_lossy(&seen);
                // Kept before the answer, so that whoever got the answer finds it heard.
                kept.lock()
                    .unwrap()
                    .push(line.lines().next().unwrap_or_default().to_owned());
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        SERVED.len()
                    )
                    .as_bytes(),
                );
                let _ = stream.write_all(&SERVED);
            }
        });
        (port, heard)
    }

    fn registry(port: u16) -> Registry {
        let text = format!(
            "[demo]\nextensions = {{ demo = \"demo\" }}\n\n[[demo.server]]\nname = \"demo\"\n\n\
             [[demo.server.download]]\nplatform = \"linux-x64\"\nversion = \"3\"\n\
             url = \"http://127.0.0.1:{port}/demo.gz\"\nsha256 = \"{DIGEST}\"\n\
             archive = \"gz\"\nprogram = \"demo\"\nsize = {}\n",
            SERVED.len()
        );
        let (registry, errors) = Registry::read(&text, None);
        assert!(errors.is_empty(), "{errors:?}");
        registry
    }

    #[test]
    fn an_empty_folder_is_filled_and_a_folder_already_filled_is_found_whole_and_fetches_nothing() {
        let folder = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("yaar-xtask-servers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let (port, heard) = serve();
        let registry = registry(port);

        let mut printed = Vec::new();
        run(&registry, Platform::LinuxX64, &folder, &mut printed).expect("the folder fills");
        let printed = String::from_utf8(printed).unwrap();
        assert_eq!(
            heard.lock().unwrap().as_slice(),
            ["GET /demo.gz HTTP/1.1"],
            "one download"
        );
        assert!(
            printed.starts_with("language-servers: demo 3: fetching http://127.0.0.1:"),
            "{printed}"
        );
        assert!(folder.join("demo/3/demo").is_file());
        assert!(folder.join("demo/installed.toml").is_file());

        let mut again = Vec::new();
        run(&registry, Platform::LinuxX64, &folder, &mut again).expect("a filled folder is whole");
        assert_eq!(
            String::from_utf8(again).unwrap(),
            "language-servers: demo 3: there already\n"
        );
        assert_eq!(heard.lock().unwrap().len(), 1, "nothing was fetched");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_download_that_fails_is_an_error_and_the_line_before_it_names_the_server() {
        let folder = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("yaar-xtask-servers-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let (port, heard) = serve();
        // The row pins a digest the listener's bytes do not have.
        let text = format!(
            "[demo]\nextensions = {{ demo = \"demo\" }}\n\n[[demo.server]]\nname = \"demo\"\n\n\
             [[demo.server.download]]\nplatform = \"linux-x64\"\nversion = \"3\"\n\
             url = \"http://127.0.0.1:{port}/demo.gz\"\nsha256 = \"{}\"\n\
             archive = \"gz\"\nprogram = \"demo\"\nsize = {}\n",
            "0".repeat(64),
            SERVED.len()
        );
        let (registry, errors) = Registry::read(&text, None);
        assert!(errors.is_empty(), "{errors:?}");

        let mut printed = Vec::new();
        let error = run(&registry, Platform::LinuxX64, &folder, &mut printed)
            .expect_err("the digest differs");
        assert!(matches!(error, DownloadError::Digest { .. }), "{error:?}");
        assert!(
            String::from_utf8(printed).unwrap().contains("demo 3"),
            "the failing server is named"
        );
        assert_eq!(heard.lock().unwrap().len(), 1);
        assert!(!folder.join("demo/installed.toml").exists());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
