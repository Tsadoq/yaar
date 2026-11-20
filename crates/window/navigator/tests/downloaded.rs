//! The three built-in rows downloaded from their publishers and used (LSP-01): with no server
//! on the machine, each row offers its download, downloads into the app's folder, passes its
//! digest, and answers hover, definition and references on its fixed project.
//!
//! This is the one test of the slice that reaches the publishers' addresses: about 40 MB
//! fetched and 90 MB unpacked a run, into the test's own scratch, removed with it. What a
//! request carries is the downloader's own (`download::USER_AGENT` and nothing that names a
//! user, a machine or a project). That a digest that differs is refused is LSP-05's.
//!
//! Each server runs with the home, cache, config, data and temporary folders and the cargo
//! home of the scratch, as the tests of the real servers run them. The waits are for facts
//! the Navigator tells, with `SERVER_DEADLINE` for a download and a real server's readiness
//! and answers, which a throttled run on a small machine takes long to reach.

mod support;

use support::*;
use yaar_navigator::download::installed;
use yaar_navigator::{Ask, BUILTIN, Location, Offer, Platform, Registry, State, Tree};

/// What one fixed project is asked: a name at a line of a file, and where the name is
/// defined and used (new side, `PLAN.md` § 3).
struct Probe {
    /// The file, relative to the worktree, and the line of the name in it, from 1.
    file: &'static str,
    line: u32,
    name: &'static str,
    /// A part of what the hover says.
    hover: &'static str,
    /// The file and line of the definition.
    defined: (&'static str, u32),
    /// Files and lines that refer to the name, the definition's among them.
    used: &'static [(&'static str, u32)],
}

/// Whether `places` holds the line `line` of `relative` in the worktree of `repo`.
fn holds(places: &[Location], repo: &Repo, relative: &str, line: u32) -> bool {
    places
        .iter()
        .any(|place| place.line == line && place.path == repo.root.join(relative))
}

/// Downloads the server of the row `language` for the fixture `fixture` and asks it each of
/// `probes`.
fn downloads_and_answers(fixture: Fixture, language: &str, server: &str, probes: &[Probe]) {
    let scratch = Scratch::new();
    let repo = scratch.fixture(fixture);
    let platform = Platform::current().expect("a machine the built-in rows download for");
    let (registry, errors) = Registry::read(BUILTIN, None);
    assert_eq!(errors, vec![]);
    let (pinned_server, pinned) = registry
        .row(language)
        .expect("a built-in row")
        .download(platform)
        .expect("the row downloads for this machine");
    assert_eq!(pinned_server.name, server);

    // With no server on the machine: the config's search path is the scratch's `bin`, which
    // holds the recorder `npm` and nothing else, and its folder of downloads is empty.
    let config = scratch.config(BUILTIN);
    let servers = config.servers.clone();
    assert_eq!(
        std::fs::read_dir(&servers).expect("the folder").count(),
        0,
        "the folder of downloads is empty"
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(config);
    running.show(&repo, &ws, &repository);
    let missing = running.wait_state(&ws, language, None, DEADLINE, |state| {
        matches!(state.state, State::Missing { .. })
    });
    assert_eq!(
        missing.state,
        State::Missing {
            offer: Some(Offer {
                server: server.to_owned(),
                version: pinned.version.clone(),
                size: pinned.size,
            })
        }
    );

    // The download, into the app's folder, and the server started from it.
    running.ask(Ask::Download {
        language: language.to_owned(),
    });
    running.wait_state(&ws, language, None, DEADLINE, |state| {
        matches!(state.state, State::Downloading { .. })
    });
    let ready = running.wait_state(&ws, language, None, SERVER_DEADLINE, |state| {
        state.state == State::Ready
    });
    let folder = servers.join(server).join(&pinned.version);
    let program = ready.program.expect("a program that runs");
    assert!(
        program.starts_with(&folder),
        "{} is not under {}",
        program.display(),
        folder.display()
    );
    assert_eq!(ready.server.as_deref(), Some(server));
    assert_eq!(ready.update, None);
    // It passed its digest: the record names the digest the row pins.
    let record = installed(&servers, server).expect("the download is installed");
    assert_eq!(record.version, pinned.version);
    assert_eq!(record.sha256, pinned.sha256);
    assert_eq!(record.program, program);

    // And it answers, hover, definition and references, from the server and not by text.
    for probe in probes {
        let at = || {
            place(
                &repo,
                &ws,
                Tree::Worktree,
                probe.file,
                probe.line,
                probe.name,
                0,
            )
        };
        let hover = running.hover(at(), SERVER_DEADLINE);
        assert!(!hover.guessed, "{} {hover:#?}", probe.file);
        let text = hover.value.expect("the server reports something");
        assert!(
            text.contains(probe.hover),
            "{} hover is {text:?}, not {:?}",
            probe.file,
            probe.hover
        );
        let definition = running.definition(at(), SERVER_DEADLINE);
        assert!(!definition.guessed, "{} {definition:#?}", probe.file);
        assert!(
            holds(&definition.value, &repo, probe.defined.0, probe.defined.1),
            "{} {definition:#?}",
            probe.file
        );
        let references = running.references(at(), SERVER_DEADLINE);
        assert!(!references.guessed, "{} {references:#?}", probe.file);
        for (file, line) in probe.used {
            assert!(
                holds(&references.value, &repo, file, *line),
                "{file}:{line} in {references:#?}"
            );
        }
    }
    running.end();
}

#[test]
fn typescript_and_javascript_are_downloaded_by_one_row_and_answer() {
    downloads_and_answers(
        Fixture::TypeScript,
        "typescript",
        "tsgo",
        &[
            Probe {
                file: "src/main.ts",
                line: 5,
                name: "perimeter",
                hover: "function perimeter(sides: number[]): number",
                defined: ("src/geometry.ts", 1),
                used: &[
                    ("src/geometry.ts", 1),
                    ("src/main.ts", 1),
                    ("src/main.ts", 5),
                ],
            },
            Probe {
                file: "src/report.js",
                line: 4,
                name: "double",
                hover: "function double(n: any): number",
                defined: ("src/helpers.js", 1),
                used: &[
                    ("src/helpers.js", 1),
                    ("src/report.js", 1),
                    ("src/report.js", 4),
                ],
            },
        ],
    );
}

#[test]
fn python_is_downloaded_and_answers() {
    downloads_and_answers(
        Fixture::Python,
        "python",
        "ty",
        &[Probe {
            file: "app/main.py",
            line: 6,
            name: "perimeter",
            hover: "def perimeter(sides: list[int]) -> int",
            defined: ("app/geometry.py", 1),
            used: &[
                ("app/geometry.py", 1),
                ("app/main.py", 1),
                ("app/main.py", 6),
            ],
        }],
    );
}

#[test]
fn rust_is_downloaded_and_answers() {
    downloads_and_answers(
        Fixture::Rust,
        "rust",
        "rust-analyzer",
        &[Probe {
            file: "src/report.rs",
            line: 5,
            name: "perimeter",
            hover: "pub fn perimeter(sides: &[u32]) -> u32",
            defined: ("src/geometry.rs", 1),
            used: &[
                ("src/geometry.rs", 1),
                ("src/report.rs", 1),
                ("src/report.rs", 5),
            ],
        }],
    );
}
