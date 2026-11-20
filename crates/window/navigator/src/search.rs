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
//! `.venv/bin/ty`); the Rust row names none, since rust-analyzer belongs to the toolchain,
//! whose program rustup puts on the search path and which runs the project's toolchain.

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
/// paths under `root` with `{platform}` expanded, its `machine` names in each folder of
/// `search_path`, and what `servers` holds for it (`crate::download::installed` of the
/// server's name). Reads the file system; runs nothing.
pub fn look(
    row: &Row,
    root: &Path,
    search_path: &[PathBuf],
    servers: &Path,
    platform: Option<Platform>,
) -> Found {
    let vars = Vars {
        root: root.display().to_string(),
        worktree: root.display().to_string(),
        platform: platform.map(|p| p.name().to_string()).unwrap_or_default(),
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
            // is what is looked for; the arguments are expanded when the server starts.
            let Ok(program) = expand(program, &vars) else {
                continue;
            };
            let program = program.as_str();
            let program = if Path::new(program).components().count() > 1
                || Path::new(program).is_absolute()
            {
                Some(root.join(program)).filter(|p| is_executable(p))
            } else {
                search_path
                    .iter()
                    .filter(|folder| folder.is_absolute())
                    .map(|folder| folder.join(program))
                    .find(|p| is_executable(p))
            };
            if let Some(program) = program {
                found
                    .registered
                    .push(candidate(CandidateKind::Registered, program, args));
            }
        }
        for path in &server.project {
            let Ok(path) = expand(path, &vars) else {
                continue;
            };
            let program = root.join(path);
            if is_executable(&program) {
                found
                    .project
                    .push(candidate(CandidateKind::Project, program, &server.args));
            }
        }
        for name in &server.machine {
            for folder in search_path.iter().filter(|folder| folder.is_absolute()) {
                let program = folder.join(name);
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
    use std::os::unix::fs::PermissionsExt;

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

    // look

    /// A folder of the test's own, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let path =
                std::env::temp_dir().join(format!("yaar-search-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            // Resolved, as the file system names it: `/tmp` is a link on macOS.
            Scratch(path.canonicalize().unwrap())
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        fn file(&self, relative: &str, mode: u32) -> PathBuf {
            let path = self.path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            path
        }

        fn executable(&self, relative: &str) -> PathBuf {
            self.file(relative, 0o755)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The program of each candidate, and its server's place in the row.
    fn programs(candidates: &[Candidate]) -> Vec<(usize, PathBuf)> {
        candidates
            .iter()
            .map(|c| (c.server, c.program.clone()))
            .collect()
    }

    const LOOK_ROW: &str = "[lang]\nextensions = { l = \"l\" }\n\
        [[lang.server]]\nname = \"one\"\nproject = [\"bin/{platform}/one\", \"bin/other/one\"]\nmachine = [\"one\", \"uno\"]\nargs = [\"--serve\"]\n\
        [[lang.server]]\nname = \"two\"\nmachine = [\"two\"]\nargs = [\"--two\"]\n";

    fn look_row() -> Registry {
        let (registry, errors) = Registry::read(LOOK_ROW, None);
        assert_eq!(errors, vec![]);
        registry
    }

    #[test]
    fn look_finds_the_projects_own_by_its_paths_with_the_platform_put_in() {
        let scratch = Scratch::new("project");
        let registry = look_row();
        let row = registry.row("lang").unwrap();
        let one = scratch.executable("bin/linux-x64/one");
        let other = scratch.executable("bin/other/one");
        // A folder of the name, and a file that is not executable, are no program.
        scratch.file("bin/darwin-arm64/one", 0o644);

        let found = look(
            row,
            &scratch.0,
            &[],
            &scratch.path("servers"),
            Some(Platform::LinuxX64),
        );
        assert_eq!(programs(&found.project), [(0, one), (0, other.clone())]);
        assert_eq!(found.project[0].kind, CandidateKind::Project);
        assert_eq!(found.project[0].args, ["--serve"]);

        let found = look(
            row,
            &scratch.0,
            &[],
            &scratch.path("servers"),
            Some(Platform::DarwinArm64),
        );
        assert_eq!(
            programs(&found.project),
            [(0, other.clone())],
            "not executable: not found"
        );

        // No platform: the variable is empty, and no path with it names a file.
        let found = look(row, &scratch.0, &[], &scratch.path("servers"), None);
        assert_eq!(programs(&found.project), [(0, other)]);
        assert!(
            found.registered.is_empty() && found.machine.is_empty() && found.downloaded.is_empty()
        );
    }

    #[test]
    fn look_finds_each_copy_on_the_machine_in_every_folder_of_the_search_path_in_its_order() {
        let scratch = Scratch::new("machine");
        let registry = look_row();
        let row = registry.row("lang").unwrap();
        let (a, b, c) = (scratch.path("a"), scratch.path("b"), scratch.path("c"));
        let one_b = scratch.executable("b/one");
        let one_a = scratch.executable("a/one");
        let uno_c = scratch.executable("c/uno");
        let two_c = scratch.executable("c/two");
        scratch.file("a/uno", 0o600);
        // A folder of the path that is not there, and one that is not an absolute path
        // (which would mean the folder the process happens to stand in).
        let search = [
            scratch.path("missing"),
            b,
            PathBuf::from("a"),
            PathBuf::new(),
            a,
            c,
        ];
        let found = look(row, &scratch.0, &search, &scratch.path("servers"), None);

        assert_eq!(
            programs(&found.machine),
            [(0, one_b), (0, one_a), (0, uno_c), (1, two_c)],
            "each name in every folder, in the path's order; a server's names in its order"
        );
        assert!(
            found
                .machine
                .iter()
                .all(|c| c.kind == CandidateKind::Machine)
        );
        assert_eq!(found.machine[0].args, ["--serve"]);
        assert_eq!(found.machine[3].args, ["--two"]);
    }

    /// A user's row of a server named `lang` with the commands written one to a line.
    fn registered(commands: &[&str]) -> Registry {
        let servers: String = commands
            .iter()
            .enumerate()
            .map(|(n, command)| format!("[[lang.server]]\nname = \"s{n}\"\ncommand = {command}\n"))
            .collect();
        let (registry, errors) = Registry::read(LOOK_ROW, Some(&format!("[lang]\n{servers}")));
        assert_eq!(errors, vec![]);
        registry
    }

    #[test]
    fn look_resolves_each_registered_command_and_leaves_out_one_that_is_not_there() {
        let scratch = Scratch::new("registered");
        let absolute = scratch.executable("abs/server");
        let bare = scratch.executable("bin/barename");
        scratch.executable("elsewhere/barename");
        let root = scratch.path("repo");
        std::fs::create_dir_all(&root).unwrap();
        let relative_in_root = scratch.executable("repo/tools/rel");

        let registry = registered(&[
            &format!("[\"{}\", \"--flag\", \"{{root}}\"]", absolute.display()),
            "[\"barename\"]",
            "[\"tools/rel\", \"x\"]",
            "[\"{root}/tools/rel\"]",
            "[\"/nowhere/at/all\"]",
            "[\"missingname\"]",
        ]);
        let row = registry.row("lang").unwrap();
        let search = [
            scratch.path("none"),
            scratch.path("bin"),
            scratch.path("elsewhere"),
        ];
        let found = look(row, &root, &search, &scratch.path("servers"), None);

        assert_eq!(
            programs(&found.registered),
            [
                (0, absolute),
                (1, bare),
                (2, relative_in_root.clone()),
                (3, relative_in_root)
            ]
        );
        assert!(
            found
                .registered
                .iter()
                .all(|c| c.kind == CandidateKind::Registered)
        );
        // The command's own arguments, not yet expanded; none for a bare program.
        assert_eq!(found.registered[0].args, ["--flag", "{root}"]);
        assert_eq!(found.registered[1].args, Vec::<String>::new());
        assert_eq!(found.registered[2].args, ["x"]);
    }

    #[test]
    fn look_leaves_out_a_registered_program_that_is_not_an_executable_file() {
        let scratch = Scratch::new("notexec");
        let plain = scratch.file("plain", 0o644);
        let folder = scratch.path("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let registry = registered(&[
            &format!("[\"{}\"]", plain.display()),
            &format!("[\"{}\"]", folder.display()),
        ]);
        let row = registry.row("lang").unwrap();
        let found = look(row, &scratch.0, &[], &scratch.path("servers"), None);
        assert_eq!(found.registered, vec![]);
    }

    #[test]
    fn what_look_finds_is_ordered_as_a_user_row_asks() {
        // The two halves meet: a row's candidates, found on disk, come out in FR-060's order.
        let scratch = Scratch::new("meet");
        let user = "[lang]\n\
            [[lang.server]]\nname = \"mine\"\ncommand = [\"mine\"]\nproject = [\"p/mine\"]\nmachine = [\"mine\"]\n";
        let (registry, errors) = Registry::read(LOOK_ROW, Some(user));
        assert_eq!(errors, vec![]);
        let row = registry.row("lang").unwrap();
        let registered = scratch.executable("bin/mine");
        let project = scratch.executable("repo/p/mine");
        let other = scratch.executable("other/mine");

        let search = [scratch.path("bin"), scratch.path("other")];
        let found = look(
            row,
            &scratch.path("repo"),
            &search,
            &scratch.path("servers"),
            None,
        );
        let tried: Vec<_> = order(row, &found)
            .into_iter()
            .map(|c| (c.kind, c.program))
            .collect();
        // `bin/mine` is the registered command and also the first `mine` of the path: once.
        assert_eq!(
            tried,
            [
                (CandidateKind::Registered, registered),
                (CandidateKind::Project, project),
                (CandidateKind::Machine, other),
            ]
        );
    }

    /// Waits for `download::installed` (P8): its body is a `todo!()` until then, and a row
    /// with a download calls it. The folder is written as `download.rs` documents it.
    #[test]
    #[ignore = "waits for download::installed, which package 8 writes"]
    fn look_finds_the_version_the_app_downloaded_for_each_server_that_has_one() {
        let scratch = Scratch::new("downloaded");
        let registry = builtin();
        let row = registry.row("python").unwrap();
        let ty = scratch.executable("servers/ty/0.0.100/ty");
        std::fs::write(
            scratch.path("servers/ty/installed.toml"),
            "version = \"0.0.100\"\nsha256 = \"22f993f68444218eca1633caaacc3c2615aa5f43122386ae811807aa7695c70a\"\nprogram = \"ty\"\n",
        )
        .unwrap();

        let found = look(
            row,
            &scratch.0,
            &[],
            &scratch.path("servers"),
            Some(Platform::DarwinArm64),
        );
        assert_eq!(programs(&found.downloaded), [(2, ty)]);
        assert_eq!(found.downloaded[0].kind, CandidateKind::Downloaded);
        assert_eq!(found.downloaded[0].args, ["server"]);

        // Nothing downloaded for the others: no folder, no candidate.
        let rust = registry.row("rust").unwrap();
        let found = look(
            rust,
            &scratch.0,
            &[],
            &scratch.path("servers"),
            Some(Platform::DarwinArm64),
        );
        assert_eq!(found.downloaded, vec![]);
    }

    // inside

    #[test]
    fn a_program_inside_a_worktree_of_the_repository_is_inside() {
        let scratch = Scratch::new("inside");
        let first = scratch.path("repo");
        let second = scratch.path("repo-second");
        let inside_first = scratch.executable("repo/tools/ls");
        let inside_second = scratch.executable("repo-second/node_modules/.bin/ls");
        let outside = scratch.executable("elsewhere/ls");
        // A folder whose name starts with the worktree's name is no part of it.
        let neighbour = scratch.executable("repository/ls");

        let worktrees = [first.as_path(), second.as_path()];
        assert!(inside(&inside_first, &worktrees));
        assert!(inside(&inside_second, &worktrees));
        assert!(!inside(&outside, &worktrees));
        assert!(!inside(&neighbour, &worktrees));
        assert!(!inside(&inside_first, &[]));
    }

    #[test]
    fn a_link_is_followed_on_both_sides_before_the_program_is_placed() {
        let scratch = Scratch::new("links");
        let repo = scratch.path("repo");
        let real = scratch.executable("repo/tools/real");
        let outside = scratch.executable("elsewhere/outside");
        std::fs::create_dir_all(scratch.path("bin")).unwrap();
        // A link outside the repository to a file inside it: inside. A link inside the
        // repository to a file outside it: outside, whatever its own place.
        std::os::unix::fs::symlink(&real, scratch.path("bin/to-inside")).unwrap();
        std::os::unix::fs::symlink(&outside, scratch.path("repo/tools/to-outside")).unwrap();
        // The worktree itself named by a link.
        std::os::unix::fs::symlink(&repo, scratch.path("repo-link")).unwrap();

        assert!(inside(&scratch.path("bin/to-inside"), &[repo.as_path()]));
        assert!(!inside(
            &scratch.path("repo/tools/to-outside"),
            &[repo.as_path()]
        ));
        assert!(inside(&real, &[scratch.path("repo-link").as_path()]));
    }

    #[test]
    fn a_program_that_is_not_there_is_placed_by_the_path_it_is_given() {
        let scratch = Scratch::new("absent");
        let repo = scratch.path("repo");
        std::fs::create_dir_all(&repo).unwrap();
        assert!(inside(&repo.join("gone/ls"), &[repo.as_path()]));
        assert!(!inside(&scratch.path("gone/ls"), &[repo.as_path()]));
    }
}
