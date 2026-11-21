//! `look` and `inside` on real files: executables, links and a folder the tests make under
//! the temporary folder, as the Navigator's search for a server reads them. What `order` does
//! with what was found is shown with no file, in the unit tests of `search.rs`.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use yaar_navigator::registry::{BUILTIN, Platform, Registry};
use yaar_navigator::search::{Candidate, CandidateKind, inside, look, order};

mod support;

fn builtin() -> Registry {
    let (registry, errors) = Registry::read(BUILTIN, None);
    assert_eq!(errors, vec![]);
    registry
}

/// A folder of the test's own, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!("yaar-search-{}-{name}", std::process::id()));
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

/// A relative path that leads from the folder the process stands in to `target`.
fn relative_to_here(target: &Path) -> PathBuf {
    let here = std::env::current_dir().unwrap().canonicalize().unwrap();
    let mut path: PathBuf = here.components().skip(1).map(|_| "..").collect();
    path.push(target.strip_prefix("/").unwrap());
    assert!(path.is_relative());
    assert_eq!(
        here.join(&path).canonicalize().unwrap(),
        target.canonicalize().unwrap(),
        "the relative path does lead there"
    );
    path
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
    let found = look(row, &scratch.0, &[], &[], &scratch.path("servers"), None);
    assert_eq!(programs(&found.project), [(0, other)]);
    assert!(found.registered.is_empty() && found.machine.is_empty() && found.downloaded.is_empty());
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
    // A folder of the path that is not there, and ones that are not absolute paths: a
    // path that does lead to `a` from where the process stands is followed by nothing,
    // since a relative folder would make whatever the process stands in a source of
    // servers.
    let search = [
        scratch.path("missing"),
        b,
        relative_to_here(&a),
        PathBuf::new(),
        a,
        c,
    ];
    let found = look(
        row,
        &scratch.0,
        &search,
        &[],
        &scratch.path("servers"),
        None,
    );

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
        "[\"onlyrelative\"]",
    ]);
    let row = registry.row("lang").unwrap();
    // A folder reached only by a relative path is no folder of the search path.
    scratch.executable("viarelative/onlyrelative");
    let search = [
        scratch.path("none"),
        relative_to_here(&scratch.path("viarelative")),
        scratch.path("bin"),
        scratch.path("elsewhere"),
    ];
    let found = look(row, &root, &search, &[], &scratch.path("servers"), None);

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
fn a_registered_program_that_cannot_be_put_in_leaves_out_only_itself() {
    let scratch = Scratch::new("unputin");
    let project = scratch.executable("bin/other/one");
    let machine = scratch.executable("onpath/one");
    let registry = look_row();
    let mut row = registry.row("lang").unwrap().clone();
    row.servers[0].command = Some(vec!["{no_such_variable}/ls".into()]);
    let found = look(
        &row,
        &scratch.0,
        &[scratch.path("onpath")],
        &[],
        &scratch.path("servers"),
        None,
    );

    assert_eq!(found.registered, vec![]);
    assert_eq!(programs(&found.project), [(0, project)]);
    assert_eq!(programs(&found.machine), [(0, machine)]);
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
    let found = look(row, &scratch.0, &[], &[], &scratch.path("servers"), None);
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
        &[],
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

/// A row with a download calls `download::installed`. The folder is written as
/// `download.rs` documents it.
#[test]
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

// Rust's project copy: rustup's own rust-analyzer (NAV-18)

/// An environment of the servers naming each variable of `pairs`.
fn environment(pairs: &[(&str, &Path)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), value.as_os_str().to_owned()))
        .collect()
}

/// What the candidates of the rust row are, as `kind program`, in the order they are tried.
fn tried_for_rust(
    scratch: &Scratch,
    root: &Path,
    search: &[PathBuf],
    environment: &[(OsString, OsString)],
) -> Vec<(CandidateKind, PathBuf)> {
    let registry = builtin();
    let row = registry.row("rust").unwrap();
    let found = look(
        row,
        root,
        search,
        environment,
        &scratch.path("servers"),
        Some(Platform::LinuxX64),
    );
    order(row, &found)
        .into_iter()
        .map(|candidate| (candidate.kind, candidate.program))
        .collect()
}

#[test]
fn rustups_rust_analyzer_found_through_the_cargo_home_is_the_projects_own_and_tried_before_a_copy_on_the_search_path()
 {
    let scratch = Scratch::new("cargo-home");
    let cargo_home = scratch.path("cargo_home");
    let proxy = scratch.executable("cargo_home/bin/rust-analyzer");
    let early = scratch.executable("early/rust-analyzer");
    let late = scratch.executable("late/rust-analyzer");
    let worktree = scratch.path("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    // Another copy of the proxy lies in the home folder: the second place, tried after it.
    let home = scratch.path("home");
    let in_home = scratch.executable("home/.cargo/bin/rust-analyzer");

    let search = [scratch.path("early"), scratch.path("late")];
    let tried = tried_for_rust(
        &scratch,
        &worktree,
        &search,
        &environment(&[("CARGO_HOME", &cargo_home), ("HOME", &home)]),
    );
    assert_eq!(
        tried,
        [
            (CandidateKind::Project, proxy.clone()),
            (CandidateKind::Project, in_home),
            (CandidateKind::Machine, early),
            (CandidateKind::Machine, late),
        ],
        "the project's own first, in the order the row names, then the machine's in the \
         search path's order"
    );
    // The proxy lies outside the worktree: no question is asked for it (`inside` is what
    // the Navigator asks), and it was not read as a path in the worktree.
    assert!(!inside(&proxy, &[worktree.as_path()]));
    assert!(!worktree.join(proxy.strip_prefix("/").unwrap()).exists());
}

#[test]
fn with_no_cargo_home_the_proxy_in_the_home_folder_is_the_projects_own() {
    let scratch = Scratch::new("home-cargo");
    let home = scratch.path("home");
    let proxy = scratch.executable("home/.cargo/bin/rust-analyzer");
    let early = scratch.executable("early/rust-analyzer");
    let worktree = scratch.path("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let search = [scratch.path("early")];

    // `CARGO_HOME` is not set: the entry that names it is passed over, and the next is used.
    let tried = tried_for_rust(
        &scratch,
        &worktree,
        &search,
        &environment(&[("HOME", &home)]),
    );
    assert_eq!(
        tried,
        [
            (CandidateKind::Project, proxy.clone()),
            (CandidateKind::Machine, early.clone())
        ]
    );

    // `CARGO_HOME` set to a folder that holds no proxy: it is passed over as well.
    let empty = scratch.path("empty_cargo_home");
    std::fs::create_dir_all(&empty).unwrap();
    let tried = tried_for_rust(
        &scratch,
        &worktree,
        &search,
        &environment(&[("CARGO_HOME", &empty), ("HOME", &home)]),
    );
    assert_eq!(
        tried,
        [
            (CandidateKind::Project, proxy),
            (CandidateKind::Machine, early)
        ]
    );
}

#[test]
fn with_no_proxy_the_machines_copy_is_first_and_no_candidate_names_a_variable() {
    let scratch = Scratch::new("no-proxy");
    let early = scratch.executable("early/rust-analyzer");
    let worktree = scratch.path("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let search = [scratch.path("early")];
    // Neither variable is set.
    let tried = tried_for_rust(&scratch, &worktree, &search, &[]);
    assert_eq!(tried, [(CandidateKind::Machine, early.clone())]);

    // Both are set, to folders that hold no proxy.
    let home = scratch.path("home");
    let cargo_home = scratch.path("cargo_home");
    let tried = tried_for_rust(
        &scratch,
        &worktree,
        &search,
        &environment(&[("CARGO_HOME", &cargo_home), ("HOME", &home)]),
    );
    assert_eq!(tried, [(CandidateKind::Machine, early)]);
    for (_, program) in &tried {
        assert!(!program.to_string_lossy().contains("{env:"), "{program:?}");
    }
}

#[test]
fn a_variable_that_is_not_set_or_is_empty_passes_its_entry_over_and_the_next_one_is_looked_at() {
    let scratch = Scratch::new("unset");
    let text = "[lang]\nextensions = { l = \"l\" }\n\
        [[lang.server]]\nname = \"one\"\n\
        project = [\"{env:NOT_SET}/bin/one\", \"{env:EMPTY}/bin/one\", \"{env:SET}/bin/one\", \"bin/one\"]\n\
        machine = [\"{env:NOT_SET}\", \"{env:SET_NAME}\"]\n";
    let (registry, errors) = Registry::read(text, None);
    assert_eq!(errors, vec![]);
    let row = registry.row("lang").unwrap();
    let set = scratch.path("set");
    // `/bin/one` is what an empty variable would make of the second entry: it must not be
    // looked at as the file system's own `/bin`.
    let in_set = scratch.executable("set/bin/one");
    let in_worktree = scratch.executable("worktree/bin/one");
    let named = scratch.executable("path/named");
    scratch.executable("path/nothing");
    let environment: Vec<(OsString, OsString)> = vec![
        ("EMPTY".into(), "".into()),
        ("SET".into(), set.into_os_string()),
        ("SET_NAME".into(), "named".into()),
    ];
    let found = look(
        row,
        &scratch.path("worktree"),
        &[scratch.path("path")],
        &environment,
        &scratch.path("servers"),
        None,
    );
    assert_eq!(programs(&found.project), [(0, in_set), (0, in_worktree)]);
    assert_eq!(programs(&found.machine), [(0, named)]);
}

#[test]
fn a_project_entry_that_is_absolute_once_expanded_is_not_joined_to_the_worktree() {
    let scratch = Scratch::new("absolute");
    let text = "[lang]\nextensions = { l = \"l\" }\n\
        [[lang.server]]\nname = \"one\"\n\
        project = [\"{env:TOOLS}/one\", \"{root}/in/one\", \"in/two\"]\n";
    let (registry, errors) = Registry::read(text, None);
    assert_eq!(errors, vec![]);
    let row = registry.row("lang").unwrap();
    let outside = scratch.executable("tools/one");
    let worktree = scratch.path("worktree");
    let in_root = scratch.executable("worktree/in/one");
    let relative = scratch.executable("worktree/in/two");
    // The same names under the worktree, which a joined path would find instead.
    scratch.executable("worktree/tools/one");
    let tools = scratch.path("tools");

    let found = look(
        row,
        &worktree,
        &[],
        &environment(&[("TOOLS", &tools)]),
        &scratch.path("servers"),
        None,
    );
    assert_eq!(
        programs(&found.project),
        [
            (0, outside.clone()),
            (0, in_root.clone()),
            (0, relative.clone())
        ]
    );
    // Only the one that lies outside asks no question: the others lie in the worktree.
    let worktrees = [worktree.as_path()];
    assert!(!inside(&outside, &worktrees));
    assert!(inside(&in_root, &worktrees));
    assert!(inside(&relative, &worktrees));
}

/// A `/bin/sh` script of the test's own at `path`.
fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_navigator_tries_rustups_proxy_before_the_machines_copy_with_no_question_and_with_no_install_asked_for()
 {
    use support::{DEADLINE, Running, repository_id, workspace_id};
    use yaar_navigator::{Ask, State, Told};

    let scratch = support::Scratch::new();
    let repo = scratch.repository(
        "project",
        &[
            ("Cargo.toml", "[package]\nname = \"p\"\n"),
            ("src/lib.rs", "pub fn f() {}\n"),
        ],
    );
    // Both copies write one line to one file and end at once, so that the file's first line
    // is the copy that was tried first. The proxy writes what it was told about installing.
    let log = scratch.root.join("rust.log");
    let proxy = scratch.root.join("cargo").join("bin").join("rust-analyzer");
    let machine = scratch.root.join("bin").join("rust-analyzer");
    let quoted = format!("'{}'", log.display());
    script(
        &proxy,
        &format!("printf 'proxy %s\\n' \"$RUSTUP_AUTO_INSTALL\" >> {quoted}\nexit 0\n"),
    );
    script(
        &machine,
        &format!("printf 'machine\\n' >> {quoted}\nexit 0\n"),
    );

    let mut config = scratch.config(yaar_navigator::BUILTIN);
    // The scratch's environment says it already; only the row may.
    config
        .environment
        .retain(|(name, _)| name != "RUSTUP_AUTO_INSTALL");
    assert!(
        config
            .environment
            .iter()
            .any(|(name, _)| name == "CARGO_HOME")
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(config);
    // No answer has been given on the servers inside the repository.
    running.ask(Ask::Consents(vec![]));
    running.show(&repo, &ws, &repository);

    // A trial run that ended with success lets the server start, and shows its process: the
    // proxy's trial has written its line by then.
    let started = running.wait_state(&ws, "rust", None, DEADLINE, |state| state.pid.is_some());
    assert_eq!(started.program, Some(proxy));
    let written = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        written.lines().next(),
        Some("proxy 0"),
        "the proxy is the first copy tried, with no install asked for: {written:?}"
    );
    // No question was shown on the way: the proxy lies outside the repository.
    for told in &running.told {
        if let Told::Servers(list) = told {
            assert!(
                !list
                    .iter()
                    .any(|state| matches!(state.state, State::Asking { .. })),
                "a question was shown: {list:#?}"
            );
        }
    }
    running.end();
}
