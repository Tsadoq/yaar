//! The three built-in rows downloaded from their publishers and used, on fixed projects.
//! Fetches about 40 MB (90 MB unpacked) a run into the scratch.

mod support;

use support::*;
use yaar_navigator::download::installed;
use yaar_navigator::{Ask, BUILTIN, Location, Offer, Platform, Registry, State, Tree};

struct Probe {
    /// Relative to the worktree; the line is 1-based.
    file: &'static str,
    line: u32,
    name: &'static str,
    hover: &'static str,
    defined: (&'static str, u32),
    used: &'static [(&'static str, u32)],
}

fn holds(places: &[Location], repo: &Repo, relative: &str, line: u32) -> bool {
    places
        .iter()
        .any(|place| place.line == line && place.path == repo.root.join(relative))
}

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
    let record = installed(&servers, server).expect("the download is installed");
    assert_eq!(record.version, pinned.version);
    assert_eq!(record.sha256, pinned.sha256);
    assert_eq!(record.program, program);

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
