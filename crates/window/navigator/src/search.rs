//! Which servers a row finds, in which order they are tried, and what is shown when none
//! is found (FR-057, FR-060, FR-062, FR-073).
//!
//! The order is FR-060's and the row's ([`order`]): the server the user registered, then
//! the project's own, then one installed on the machine, then one the app downloaded, each
//! kind in the order the row names its servers (ADR-015 point 5). [`order`] is a pure function of
//! what [`look`] found, so that the rule is shown with no file and no process (NAV-07).
//!
//! What "the project's own" is, each server of a row says (`project`: paths under the
//! worktree's root, as `node_modules/@typescript/typescript-{platform}/lib/tsc` or
//! `.venv/bin/ty`, or absolute paths outside it, which a variable `{env:NAME}` of the servers'
//! environment names: the Rust row names rustup's proxy in the cargo folder, which runs the
//! rust-analyzer of the toolchain the project pins). An entry whose variable is not set is
//! passed over.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::download::installed;
use crate::port::{Offer, State};
use crate::registry::{Platform, Row, Vars, expand};

/// Where a candidate comes from, in the order they are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    /// A command the user registered in their own file.
    Registered,
    /// A path of a server's `project` under the worktree's root.
    Project,
    /// A name of a server's `machine` found in the search path.
    Machine,
    /// A server the app downloaded.
    Downloaded,
}

/// One copy of a server a row may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Where it comes from.
    pub kind: CandidateKind,
    /// Which server of the row it is a copy of, by its place in `Row::servers`: its `args`,
    /// `trial`, `ready` and settings are that server's.
    pub server: usize,
    /// Its program, by its absolute path.
    pub program: PathBuf,
    /// The arguments it is started with: a registered command's own, else its server's
    /// `args`, not yet expanded.
    pub args: Vec<String>,
}

/// What [`look`] found of a row's servers, each kind in the order of the row's servers and,
/// within one server, in the order of its paths or of the search path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// The registered commands, their programs resolved: an absolute path as it is, a bare
    /// name looked for in the search path, a relative path joined to the worktree's root;
    /// one whose program is not there is left out.
    pub registered: Vec<Candidate>,
    /// Each path of a server's `project` that is an executable file.
    pub project: Vec<Candidate>,
    /// Each file named by a server's `machine` that is an executable file of a folder of the
    /// search path: every such folder, in the path's order.
    pub machine: Vec<Candidate>,
    /// The program of the version the app downloaded and uses, for each server that has
    /// one.
    pub downloaded: Vec<Candidate>,
}

/// The candidates of `row`, in the order they are tried (FR-060): every registered one,
/// then every project's own, then every one installed on the machine, then every one the
/// app downloaded, each kind in [`Found`]'s order. A program found twice is tried once, at
/// its first place. A user's row has no downloaded candidate, whatever `found` holds: it
/// replaces the built-in row whole and is never downloaded (FR-073).
pub fn order(row: &Row, found: &Found) -> Vec<Candidate> {
    let mut tried: Vec<Candidate> = Vec::new();
    let kinds = [
        &found.registered,
        &found.project,
        &found.machine,
        &found.downloaded,
    ];
    for (n, kind) in kinds.into_iter().enumerate() {
        // The user's row replaces the built-in row whole, downloads included.
        if row.registered && n == 3 {
            continue;
        }
        // A stable sort: within one server the paths keep the order they were found in.
        let mut kind: Vec<&Candidate> = kind.iter().collect();
        kind.sort_by_key(|candidate| candidate.server);
        for candidate in kind {
            if !tried.iter().any(|t| t.program == candidate.program) {
                tried.push(candidate.clone());
            }
        }
    }
    tried
}

/// Looks for the candidates of `row`: the registered commands, each server's `project`
/// paths under `root` (or outside it, where one is absolute once expanded) with `{platform}`
/// and `{env:NAME}` expanded against `environment`, its `machine` names in each folder of
/// `search_path`, and what `servers` holds for it (`crate::download::installed` of the
/// server's name). An entry that names a variable `environment` does not hold is passed over.
/// Reads the file system; runs nothing.
pub fn look(
    row: &Row,
    root: &Path,
    search_path: &[PathBuf],
    environment: &[(OsString, OsString)],
    servers: &Path,
    platform: Option<Platform>,
) -> Found {
    let vars = Vars {
        root: root.display().to_string(),
        worktree: root.display().to_string(),
        platform: platform.map(|p| p.name().to_string()).unwrap_or_default(),
        env: Some(environment.to_vec()),
        ..Vars::default()
    };
    let mut found = Found::default();
    for (at, server) in row.servers.iter().enumerate() {
        let candidate = |kind, program: PathBuf, args: &[String]| Candidate {
            kind,
            server: at,
            program,
            args: args.to_vec(),
        };
        if let Some((program, args)) = server.command.as_ref().and_then(|c| c.split_first()) {
            // A variable in the program (`{root}/tools/ls`) is put in here, as the program
            // is what is looked for; the arguments are expanded when the server starts. A
            // program that cannot be put in is no candidate, and the server's other paths
            // are still looked at.
            let program = expand(program, &vars).ok().and_then(|program| {
                if Path::new(&program).components().count() > 1 || Path::new(&program).is_absolute()
                {
                    Some(root.join(&program)).filter(|p| is_executable(p))
                } else {
                    search_path
                        .iter()
                        .filter(|folder| folder.is_absolute())
                        .map(|folder| folder.join(&program))
                        .find(|p| is_executable(p))
                }
            });
            if let Some(program) = program {
                found
                    .registered
                    .push(candidate(CandidateKind::Registered, program, args));
            }
        }
        for path in &server.project {
            // An entry that names a variable that is not set is passed over. One that is
            // absolute once expanded names a file outside the worktree, which is not joined to
            // it: rustup's proxy in the user's cargo folder is no part of the repository.
            let Ok(path) = expand(path, &vars) else {
                continue;
            };
            let program = if Path::new(&path).is_absolute() {
                PathBuf::from(path)
            } else {
                root.join(path)
            };
            if is_executable(&program) {
                found
                    .project
                    .push(candidate(CandidateKind::Project, program, &server.args));
            }
        }
        for name in &server.machine {
            let Ok(name) = expand(name, &vars) else {
                continue;
            };
            for folder in search_path.iter().filter(|folder| folder.is_absolute()) {
                let program = folder.join(&name);
                if is_executable(&program) {
                    found
                        .machine
                        .push(candidate(CandidateKind::Machine, program, &server.args));
                }
            }
        }
        // Only a server with a download can have one installed; a user's row has none.
        if !server.downloads.is_empty()
            && let Some(installed) = installed(servers, &server.name)
        {
            found.downloaded.push(candidate(
                CandidateKind::Downloaded,
                installed.program,
                &server.args,
            ));
        }
    }
    found
}

/// The offer to download `row`'s server on `platform`: the first server of the row with a
/// download for that platform (`Row::download`); none for a registered row, for a row with
/// no download for that platform, and where the platform is none.
pub fn offer(row: &Row, platform: Option<Platform>) -> Option<Offer> {
    let (server, download) = row.download(platform?)?;
    Some(Offer {
        server: server.name.clone(),
        version: download.version.clone(),
        size: download.size,
    })
}

/// The state shown when no candidate of `row` is left (NAV-06): missing, with
/// [`offer`]'s offer.
pub fn when_none(row: &Row, platform: Option<Platform>) -> State {
    State::Missing {
        offer: offer(row, platform),
    }
}

/// Whether `program`, its links resolved, lies inside one of `worktrees`, the folders of
/// the worktrees of one repository, theirs resolved too (FR-062).
pub fn inside(program: &Path, worktrees: &[&Path]) -> bool {
    let program = resolved(program);
    worktrees
        .iter()
        .any(|worktree| program.starts_with(resolved(worktree)))
}

/// `path` with its links resolved; as it is where it cannot be (a program that is not there
/// is placed by the path it was given).
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `path` is a file, after links, that anyone may execute.
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{BUILTIN, Registry};

    fn builtin() -> Registry {
        let (registry, errors) = Registry::read(BUILTIN, None);
        assert_eq!(errors, vec![]);
        registry
    }

    // NAV-06

    #[test]
    fn a_built_in_row_with_no_server_found_is_missing_with_the_offer_of_its_first_download() {
        let registry = builtin();
        // The server each row offers is the first of its servers that has a download: the
        // third of Python's four, with two before it that have none.
        let first_with_a_download = [
            ("typescript", "tsgo"),
            ("python", "ty"),
            ("rust", "rust-analyzer"),
        ];
        assert_eq!(registry.rows().len(), first_with_a_download.len());

        for (name, server) in first_with_a_download {
            let row = registry.row(name).unwrap();
            for platform in Platform::ALL {
                let download = row
                    .servers
                    .iter()
                    .find(|s| s.name == server)
                    .and_then(|s| s.download(platform))
                    .unwrap_or_else(|| panic!("{server} has no download for {}", platform.name()));
                assert_eq!(
                    when_none(row, Some(platform)),
                    State::Missing {
                        offer: Some(Offer {
                            server: server.to_string(),
                            version: download.version.clone(),
                            size: download.size,
                        })
                    },
                    "{name} on {}",
                    platform.name()
                );
                assert_eq!(offer(row, Some(platform)).unwrap().server, server);
            }
        }
    }

    #[test]
    fn the_offer_names_the_server_version_and_size_the_row_pins_for_the_platform() {
        let digest = "0".repeat(64);
        let download = |platform: &str, version: &str, size: u64| {
            format!(
                "[[lang.server.download]]\nplatform = \"{platform}\"\nversion = \"{version}\"\n\
                 url = \"https://e.org/x\"\nsha256 = \"{digest}\"\narchive = \"gz\"\n\
                 program = \"x\"\nsize = {size}\n"
            )
        };
        // The first server has none for linux-x64, the second has; the third has one too and
        // is not the offer.
        let text = format!(
            "[lang]\nextensions = {{ l = \"l\" }}\n\
             [[lang.server]]\nname = \"none\"\n\
             [[lang.server]]\nname = \"mac-only\"\n{}\
             [[lang.server]]\nname = \"both\"\n{}{}\
             [[lang.server]]\nname = \"third\"\n{}",
            download("darwin-arm64", "1.0", 11),
            download("darwin-arm64", "2.0", 22),
            download("linux-x64", "2.1", 33),
            download("linux-x64", "3.0", 44),
        );
        let (registry, errors) = Registry::read(&text, None);
        assert_eq!(errors, vec![]);
        let row = registry.row("lang").unwrap();

        let want = |server: &str, version: &str, size: u64| {
            Some(Offer {
                server: server.into(),
                version: version.into(),
                size,
            })
        };
        assert_eq!(
            offer(row, Some(Platform::DarwinArm64)),
            want("mac-only", "1.0", 11)
        );
        assert_eq!(
            offer(row, Some(Platform::LinuxX64)),
            want("both", "2.1", 33)
        );
        assert_eq!(offer(row, Some(Platform::LinuxArm64)), None);
        assert_eq!(
            when_none(row, Some(Platform::LinuxX64)),
            State::Missing {
                offer: want("both", "2.1", 33)
            }
        );
    }

    #[test]
    fn a_supported_language_on_a_machine_with_no_download_has_no_offer() {
        let registry = builtin();
        for row in registry.rows() {
            assert_eq!(
                when_none(row, None),
                State::Missing { offer: None },
                "{}",
                row.name
            );
            assert_eq!(offer(row, None), None);
        }
    }

    #[test]
    fn a_row_the_user_registered_is_missing_with_no_offer_whatever_the_platform() {
        let user = "[rust]\n[[rust.server]]\ncommand = [\"/nowhere/ra\"]\n\
                    [standin]\nextensions = { sil = \"standin\" }\n\
                    [[standin.server]]\nname = \"mine\"\ncommand = [\"/nowhere/s\"]\n";
        let (registry, errors) = Registry::read(BUILTIN, Some(user));
        assert_eq!(errors, vec![]);

        for name in ["rust", "standin"] {
            let row = registry.row(name).unwrap();
            for platform in Platform::ALL.map(Some).into_iter().chain([None]) {
                assert_eq!(
                    when_none(row, platform),
                    State::Missing { offer: None },
                    "{name} on {platform:?}"
                );
            }
        }
    }

    #[test]
    fn a_row_flagged_as_the_users_is_offered_no_download_even_when_a_server_of_it_has_one() {
        // The reader never lets a user's file carry a download, so a row is built by hand
        // from a built-in one: its servers keep their downloads, the row says it is the
        // user's.
        let registry = builtin();
        for name in ["typescript", "python", "rust"] {
            let mut row = registry.row(name).unwrap().clone();
            assert!(
                row.servers.iter().any(|s| !s.downloads.is_empty()),
                "{name}"
            );
            row.registered = true;
            for platform in Platform::ALL.map(Some).into_iter().chain([None]) {
                assert_eq!(offer(&row, platform), None, "{name} on {platform:?}");
                assert_eq!(
                    when_none(&row, platform),
                    State::Missing { offer: None },
                    "{name} on {platform:?}"
                );
            }
        }
    }

    #[test]
    fn a_file_of_no_rows_language_has_no_row_and_so_no_state() {
        let registry = builtin();
        assert!(registry.for_path(Path::new("notes.txt")).is_none());
        assert!(registry.for_path(Path::new("Makefile")).is_none());
        assert_eq!(
            registry.for_path(Path::new("src/lib.rs")).unwrap().name,
            "rust"
        );
    }

    // NAV-07

    fn candidate(kind: CandidateKind, server: usize, program: &str, args: &[&str]) -> Candidate {
        Candidate {
            kind,
            server,
            program: PathBuf::from(program),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    /// What a list of candidates is, as `kind server program`, for reading a failure.
    fn shown(candidates: &[Candidate]) -> Vec<String> {
        candidates
            .iter()
            .map(|c| format!("{:?} {} {}", c.kind, c.server, c.program.display()))
            .collect()
    }

    #[test]
    fn of_several_servers_the_registered_comes_first_then_the_projects_then_the_machines_then_the_downloaded()
     {
        use CandidateKind::*;
        let registry = builtin();
        let python = registry.row("python").unwrap();
        let names: Vec<_> = python.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["basedpyright", "pyright", "ty", "pyrefly"]);

        let found = Found {
            registered: vec![],
            project: vec![
                candidate(
                    Project,
                    0,
                    "/w/.venv/bin/basedpyright-langserver",
                    &["--stdio"],
                ),
                candidate(Project, 2, "/w/.venv/bin/ty", &["server"]),
            ],
            machine: vec![
                candidate(Machine, 2, "/usr/local/bin/ty", &["server"]),
                candidate(Machine, 2, "/opt/bin/ty", &["server"]),
                candidate(Machine, 3, "/usr/local/bin/pyrefly", &["lsp"]),
            ],
            downloaded: vec![candidate(
                Downloaded,
                2,
                "/app/servers/ty/0.0.100/ty",
                &["server"],
            )],
        };

        assert_eq!(
            order(python, &found),
            vec![
                candidate(
                    Project,
                    0,
                    "/w/.venv/bin/basedpyright-langserver",
                    &["--stdio"]
                ),
                candidate(Project, 2, "/w/.venv/bin/ty", &["server"]),
                candidate(Machine, 2, "/usr/local/bin/ty", &["server"]),
                candidate(Machine, 2, "/opt/bin/ty", &["server"]),
                candidate(Machine, 3, "/usr/local/bin/pyrefly", &["lsp"]),
                candidate(Downloaded, 2, "/app/servers/ty/0.0.100/ty", &["server"]),
            ],
            "the first is what is tried first"
        );
    }

    #[test]
    fn within_a_kind_the_order_is_the_rows_order_of_servers_then_the_paths_order() {
        use CandidateKind::*;
        let registry = builtin();
        let python = registry.row("python").unwrap();
        // Handed over out of the row's order: pyrefly (3) before ty (2) before
        // basedpyright (0); within ty the two folders keep their order.
        let found = Found {
            registered: vec![],
            project: vec![],
            machine: vec![
                candidate(Machine, 3, "/a/pyrefly", &[]),
                candidate(Machine, 2, "/b/ty", &[]),
                candidate(Machine, 0, "/z/basedpyright-langserver", &[]),
                candidate(Machine, 2, "/a/ty", &[]),
            ],
            downloaded: vec![
                candidate(Downloaded, 3, "/d/pyrefly", &[]),
                candidate(Downloaded, 0, "/d/based", &[]),
            ],
        };
        assert_eq!(
            shown(&order(python, &found)),
            [
                "Machine 0 /z/basedpyright-langserver",
                "Machine 2 /b/ty",
                "Machine 2 /a/ty",
                "Machine 3 /a/pyrefly",
                "Downloaded 0 /d/based",
                "Downloaded 3 /d/pyrefly",
            ]
        );
    }

    #[test]
    fn kinds_that_are_absent_are_skipped_and_nothing_found_is_nothing_to_try() {
        use CandidateKind::*;
        let registry = builtin();
        let rust = registry.row("rust").unwrap();
        assert_eq!(order(rust, &Found::default()), vec![]);

        let found = Found {
            machine: vec![candidate(Machine, 0, "/m/rust-analyzer", &[])],
            downloaded: vec![candidate(Downloaded, 0, "/d/rust-analyzer", &[])],
            ..Found::default()
        };
        assert_eq!(
            shown(&order(rust, &found)),
            [
                "Machine 0 /m/rust-analyzer",
                "Downloaded 0 /d/rust-analyzer"
            ]
        );
    }

    #[test]
    fn a_program_found_twice_is_tried_once_at_its_first_place() {
        use CandidateKind::*;
        let registry = builtin();
        let python = registry.row("python").unwrap();
        // The project's own `ty` is also on the search path, as when `.venv/bin` is on it;
        // and the downloaded one is the same file as a machine one.
        let found = Found {
            registered: vec![],
            project: vec![candidate(Project, 2, "/w/.venv/bin/ty", &["server"])],
            machine: vec![
                candidate(Machine, 2, "/w/.venv/bin/ty", &["server"]),
                candidate(Machine, 2, "/usr/bin/ty", &["server"]),
                candidate(Machine, 2, "/usr/bin/ty", &["server"]),
            ],
            downloaded: vec![candidate(Downloaded, 2, "/usr/bin/ty", &["server"])],
        };
        assert_eq!(
            shown(&order(python, &found)),
            ["Project 2 /w/.venv/bin/ty", "Machine 2 /usr/bin/ty"]
        );
    }

    /// A user's `python` row: two registered commands, the second with a `project` of its
    /// own, a third server of a command and a `machine`.
    fn users_python() -> Registry {
        let user = "[python]\n\
            [[python.server]]\nname = \"first\"\ncommand = [\"/opt/first\", \"--a\", \"{root}\"]\n\
            [[python.server]]\nname = \"second\"\ncommand = [\"/opt/second\"]\nproject = [\"tools/second\"]\nargs = [\"--b\"]\n\
            [[python.server]]\nname = \"third\"\ncommand = [\"/opt/third\"]\nmachine = [\"third\"]\n";
        let (registry, errors) = Registry::read(BUILTIN, Some(user));
        assert_eq!(errors, vec![]);
        registry
    }

    #[test]
    fn a_users_row_tries_what_the_user_registered_first_and_downloads_nothing() {
        use CandidateKind::*;
        let registry = users_python();
        let python = registry.row("python").unwrap();
        assert!(python.registered);

        let found = Found {
            registered: vec![
                candidate(Registered, 0, "/opt/first", &["--a", "{root}"]),
                candidate(Registered, 1, "/opt/second", &[]),
                candidate(Registered, 2, "/opt/third", &[]),
            ],
            project: vec![candidate(Project, 1, "/w/tools/second", &["--b"])],
            machine: vec![candidate(Machine, 2, "/bin/third", &[])],
            // Whatever the folder holds: the user's row replaces the built-in row whole.
            downloaded: vec![candidate(
                Downloaded,
                2,
                "/app/servers/ty/1/ty",
                &["server"],
            )],
        };
        let order = order(python, &found);
        assert_eq!(
            shown(&order),
            [
                "Registered 0 /opt/first",
                "Registered 1 /opt/second",
                "Registered 2 /opt/third",
                "Project 1 /w/tools/second",
                "Machine 2 /bin/third",
            ]
        );
        // The registered ones go with their commands' own arguments.
        assert_eq!(order[0].args, ["--a", "{root}"]);
        assert_eq!(order[3].args, ["--b"]);
    }
}
