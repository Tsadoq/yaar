//! Old side of a diff answered by the real servers (tsgo, ty, rust-analyzer) from
//! `YAAR_LANGUAGE_SERVERS`, each with its test's scratch as home, cache, config, data,
//! temporary and cargo home.

mod support;

use std::path::Path;

use support::*;
use yaar_navigator::{
    Answered, Ask, AskId, BUILTIN, ExportAsk, Location, ServerState, State, Told, Tree,
};

enum Defined {
    InTheExport(&'static str, u32),
    /// In a dependency: a file whose path ends so, at this line.
    InADependency(&'static str, u32),
}

struct Probe {
    file: &'static str,
    /// The line, from 1.
    line: u32,
    name: &'static str,
    hover: &'static str,
    defined: Defined,
    /// A file of the base the change took away, which the worktree no longer has, when the
    /// definition is in one.
    gone: Option<&'static str>,
}

fn answer_of(running: &mut Running, id: u64) -> Answered<Option<String>> {
    let is_it = |told: &Told| matches!(told, Told::Hover { id: got, .. } if *got == AskId(id));
    let told = match running.told.iter().find(|told| is_it(told)) {
        Some(told) => told.clone(),
        None => running.wait_told("the answer to a hover", SERVER_DEADLINE, is_it),
    };
    match told {
        Told::Hover { answer, .. } => answer,
        _ => unreachable!("only the answer to the ask satisfies the wait"),
    }
}

fn old_side_ready(
    running: &mut Running,
    workspace: &yaar_link::WorkspaceId,
    language: &str,
    version: &str,
) -> ServerState {
    let of_the_old_side = |state: &&ServerState| {
        state.workspace == *workspace
            && state.language == language
            && state.old_side.as_deref() == Some(version)
    };
    let read = running.told.iter().find_map(|told| match told {
        Told::Servers(states) => states
            .iter()
            .filter(of_the_old_side)
            .find(|state| state.state == State::Ready)
            .cloned(),
        _ => None,
    });
    match read {
        Some(state) => state,
        None => running.wait_state(
            workspace,
            language,
            Some(version),
            SERVER_DEADLINE,
            |state| state.state == State::Ready,
        ),
    }
}

fn holds(places: &[Location], path: &Path, line: u32) -> bool {
    places
        .iter()
        .any(|place| place.line == line && place.path == path)
}

fn holds_ending(places: &[Location], ending: &str, line: u32) -> bool {
    places
        .iter()
        .any(|place| place.line == line && place.path.ends_with(ending))
}

fn answers_from_the_base(fixture: Fixture, language: &str, probes: &[Probe], linked: &[&str]) {
    let servers = real_servers();
    let scratch = Scratch::new();
    let repo = scratch.fixture(fixture);
    let mut config = scratch.config(BUILTIN);
    config.servers = servers;
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(config);
    running.show(&repo, &ws, &repository);

    // The first ask about the old side is answered at once by text match, marked as guessed;
    // the export is written meanwhile.
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: Some(repo.base.clone()),
    });
    let at = |probe: &Probe| {
        place(
            &repo,
            &ws,
            Tree::Version(repo.base.clone()),
            probe.file,
            probe.line,
            probe.name,
            0,
        )
    };
    running.ask(Ask::Hover {
        id: AskId(1),
        place: at(&probes[0]),
    });
    let export: ExportAsk = running.export_when_wanted(&repo, DEADLINE);
    assert_eq!(export.version, repo.base);
    let meanwhile = answer_of(&mut running, 1);
    assert!(
        meanwhile.guessed,
        "the old side answered before it was ready: {meanwhile:#?}"
    );
    old_side_ready(&mut running, &ws, language, &repo.base);

    for folder in linked {
        let link = export.into.join(folder);
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the export holds the folder")
                .file_type()
                .is_symlink(),
            "{folder} of the export is no link"
        );
        assert_eq!(
            std::fs::read_link(&link).expect("read the link"),
            repo.root.join(folder),
            "{folder} of the export does not point at the worktree's"
        );
    }

    for probe in probes {
        let hover = running.hover(at(probe), SERVER_DEADLINE);
        assert!(!hover.guessed, "{}: {hover:#?}", probe.name);
        let text = hover
            .value
            .unwrap_or_else(|| panic!("the server reports nothing for {}", probe.name));
        assert!(
            text.contains(probe.hover),
            "the hover of {} lacks {:?}: {text}",
            probe.name,
            probe.hover
        );

        let definition = running.definition(at(probe), SERVER_DEADLINE);
        assert!(!definition.guessed, "{}: {definition:#?}", probe.name);
        match &probe.defined {
            Defined::InTheExport(file, line) => assert!(
                holds(&definition.value, &export.into.join(file), *line),
                "{} is not defined at line {line} of {file} of the export: {definition:#?}",
                probe.name
            ),
            Defined::InADependency(ending, line) => assert!(
                holds_ending(&definition.value, ending, *line),
                "{} is not defined at line {line} of a file ending {ending}: {definition:#?}",
                probe.name
            ),
        }
        if let Some(gone) = probe.gone {
            assert!(
                !repo.root.join(gone).exists(),
                "{gone} is still in the worktree: the probe does not show the old side"
            );
            assert!(export.into.join(gone).is_file());
        }
    }
    running.end();
    assert!(!export.into.exists(), "the export outlived the Navigator");
}

#[test]
fn typescript_hover_and_definition_on_a_removed_line_answer_from_the_base_for_a_deleted_symbol_and_a_moved_one()
 {
    answers_from_the_base(
        Fixture::TypeScript,
        "typescript",
        &[
            Probe {
                file: "src/main.ts",
                line: 6,
                name: "legacyScale",
                hover: "function legacyScale(sides: number[], factor: number): number[]",
                defined: Defined::InTheExport("src/geometry.ts", 5),
                gone: None,
            },
            Probe {
                file: "src/main.ts",
                line: 8,
                name: "titleCase",
                hover: "function titleCase(word: string): string",
                defined: Defined::InTheExport("src/strings.ts", 1),
                gone: Some("src/strings.ts"),
            },
        ],
        &["node_modules"],
    );
}

#[test]
fn rust_hover_and_definition_on_a_removed_line_answer_from_the_base_for_a_deleted_symbol_and_a_moved_one()
 {
    answers_from_the_base(
        Fixture::Rust,
        "rust",
        &[
            Probe {
                file: "src/report.rs",
                line: 5,
                name: "legacy_scale",
                hover: "pub fn legacy_scale(sides: &[u32], factor: u32) -> Vec<u32>",
                defined: Defined::InTheExport("src/geometry.rs", 5),
                gone: None,
            },
            Probe {
                file: "src/report.rs",
                line: 6,
                name: "title_case",
                hover: "pub fn title_case(word: &str) -> String",
                defined: Defined::InTheExport("src/strings.rs", 1),
                gone: Some("src/strings.rs"),
            },
        ],
        &[],
    );
}

#[test]
fn typescript_a_type_from_a_dependency_on_a_removed_line_resolves() {
    answers_from_the_base(
        Fixture::TypeScript,
        "typescript",
        &[Probe {
            file: "src/main.ts",
            line: 7,
            name: "Shape",
            hover: "interface Shape",
            defined: Defined::InADependency("node_modules/tinyshape/index.d.ts", 1),
            gone: None,
        }],
        &["node_modules"],
    );
}

#[test]
fn python_a_type_from_a_dependency_on_a_removed_line_resolves() {
    answers_from_the_base(
        Fixture::Python,
        "python",
        &[Probe {
            file: "app/main.py",
            line: 8,
            name: "Shape",
            hover: "class Shape",
            defined: Defined::InADependency("site-packages/tinyshape/__init__.py", 1),
            gone: None,
        }],
        &[".venv"],
    );
}
