//! What the real servers (tsgo, ty, rust-analyzer) leave on the machine. No tracer runs: a
//! connection opened without the proxy variables is not observed.

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use support::*;
use yaar_navigator::{BUILTIN, Location, State, Tree};

struct Probe {
    language: &'static str,
    file: &'static str,
    line: u32,
    name: &'static str,
    signature: &'static str,
    defined: (&'static str, u32),
    used: &'static [(&'static str, u32)],
}

const TYPESCRIPT: Probe = Probe {
    language: "typescript",
    file: "src/main.ts",
    line: 5,
    name: "perimeter",
    signature: "function perimeter(sides: number[]): number",
    defined: ("src/geometry.ts", 1),
    used: &[
        ("src/geometry.ts", 1),
        ("src/main.ts", 1),
        ("src/main.ts", 5),
    ],
};

const JAVASCRIPT: Probe = Probe {
    language: "typescript",
    file: "src/report.js",
    line: 4,
    name: "double",
    signature: "function double(n: any): number",
    defined: ("src/helpers.js", 1),
    used: &[
        ("src/helpers.js", 1),
        ("src/report.js", 1),
        ("src/report.js", 4),
    ],
};

const PYTHON: Probe = Probe {
    language: "python",
    file: "app/main.py",
    line: 6,
    name: "perimeter",
    signature: "def perimeter(sides: list[int]) -> int",
    defined: ("app/geometry.py", 1),
    used: &[
        ("app/geometry.py", 1),
        ("app/main.py", 1),
        ("app/main.py", 6),
    ],
};

const RUST: Probe = Probe {
    language: "rust",
    file: "src/report.rs",
    line: 5,
    name: "perimeter",
    signature: "pub fn perimeter(sides: &[u32]) -> u32",
    defined: ("src/geometry.rs", 1),
    used: &[
        ("src/geometry.rs", 1),
        ("src/report.rs", 1),
        ("src/report.rs", 5),
    ],
};

fn holds(places: &[Location], repo: &Repo, relative: &str, line: u32) -> bool {
    places
        .iter()
        .any(|place| place.line == line && place.path == repo.root.join(relative))
}

fn wait_for_npm_install(scratch: &Scratch) -> Vec<String> {
    let until = Instant::now() + SERVER_DEADLINE;
    loop {
        let ran = scratch.npm_ran();
        if ran.iter().any(|line| line.contains("install")) {
            return ran;
        }
        assert!(
            Instant::now() < until,
            "waited {SERVER_DEADLINE:?} for tsgo to run npm install; the recorder holds {ran:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn leaves_nothing_behind(fixture: Fixture, probes: &[Probe]) {
    let servers = real_servers();
    let scratch = Scratch::new();
    let repo = scratch.fixture(fixture);
    let before = repo.status();
    assert!(
        !repo.root.join("target").exists(),
        "the project has a build folder"
    );

    let mut config = scratch.config(BUILTIN);
    config.servers = servers;
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(config);
    running.show(&repo, &ws, &repository);
    for language in probes
        .iter()
        .map(|probe| probe.language)
        .collect::<BTreeSet<_>>()
    {
        running.wait_state(&ws, language, None, SERVER_DEADLINE, |state| {
            state.state == State::Ready
        });
    }

    if matches!(fixture, Fixture::TypeScript) {
        let outside = running.hover(
            place(
                &repo,
                &ws,
                Tree::Worktree,
                "tools/build.js",
                3,
                "readFileSync",
                0,
            ),
            SERVER_DEADLINE,
        );
        assert!(!outside.guessed, "{outside:#?}");
    }

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
        assert!(!hover.guessed, "{hover:#?}");
        let text = hover
            .value
            .unwrap_or_else(|| panic!("the server reports something for {}", probe.name));
        assert!(text.contains(probe.signature), "{text}");
        let definition = running.definition(at(), SERVER_DEADLINE);
        assert!(!definition.guessed, "{definition:#?}");
        assert!(
            holds(&definition.value, &repo, probe.defined.0, probe.defined.1),
            "{definition:#?}"
        );
        let references = running.references(at(), SERVER_DEADLINE);
        assert!(!references.guessed, "{references:#?}");
        for (file, line) in probe.used {
            assert!(
                holds(&references.value, &repo, file, *line),
                "{file}:{line} in {references:#?}"
            );
        }
    }
    running.end();

    assert_eq!(repo.status(), before, "git status shows something new");
    assert!(
        !repo.root.join("target").exists(),
        "a build folder came into the project"
    );
    if matches!(fixture, Fixture::Rust) {
        let own: PathBuf = scratch
            .root
            .join("navigator")
            .join("targets")
            .join(ws.as_str())
            .join("new");
        assert!(
            own.is_dir(),
            "the server's own build folder {} is not there",
            own.display()
        );
    }
    assert_eq!(scratch.npm_ran(), Vec::<String>::new());
    assert_eq!(scratch.left_outside(), Vec::<PathBuf>::new());
    assert_eq!(scratch.proxy_heard(), Vec::new());
}

#[test]
fn typescript_leaves_nothing_in_the_repository_and_runs_no_package_manager() {
    leaves_nothing_behind(Fixture::TypeScript, &[TYPESCRIPT, JAVASCRIPT]);
}

#[test]
fn python_leaves_nothing_in_the_repository_and_runs_no_package_manager() {
    leaves_nothing_behind(Fixture::Python, &[PYTHON]);
}

#[test]
fn rust_leaves_nothing_in_the_repository_and_builds_into_the_apps_cache() {
    leaves_nothing_behind(Fixture::Rust, &[RUST]);
}

fn builtin_without_the_setting() -> String {
    let mut kept = Vec::new();
    let mut removed = 0;
    let mut lines = BUILTIN.lines();
    while let Some(line) = lines.next() {
        if line
            .trim_start()
            .starts_with("[typescript.server.settings.")
            && line
                .trim_end()
                .ends_with(".tsserver.automaticTypeAcquisition]")
        {
            let switch = lines.next().expect("a line under the header");
            assert_eq!(switch.trim(), "enabled = false", "under {line}");
            removed += 2;
        } else {
            kept.push(line);
        }
    }
    assert_eq!(
        removed, 6,
        "the built-in row no longer holds the three tables of the setting as this test reads them"
    );
    kept.join("\n") + "\n"
}

#[test]
fn typescript_without_its_setting_runs_npm_for_a_file_outside_every_project() {
    let servers = real_servers();
    let scratch = Scratch::new();
    let repo = scratch.fixture(Fixture::TypeScript);
    let mut config = scratch.config(&builtin_without_the_setting());
    config.servers = servers;
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(config);
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "typescript", None, SERVER_DEADLINE, |state| {
        state.state == State::Ready
    });
    let outside = running.hover(
        place(
            &repo,
            &ws,
            Tree::Worktree,
            "tools/build.js",
            3,
            "readFileSync",
            0,
        ),
        SERVER_DEADLINE,
    );
    assert!(!outside.guessed, "{outside:#?}");

    // Positive control: without the setting tsgo does run npm.
    let ran = wait_for_npm_install(&scratch);
    assert!(ran.iter().any(|line| line.contains("install")), "{ran:?}");
    running.end();
}
