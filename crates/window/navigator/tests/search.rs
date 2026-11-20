//! `look` and `inside` on real files: executables, links and a folder the tests make under
//! the temporary folder, as the Navigator's search for a server reads them. What `order` does
//! with what was found is shown with no file, in the unit tests of `search.rs`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use yaar_navigator::registry::{BUILTIN, Platform, Registry};
use yaar_navigator::search::{Candidate, CandidateKind, inside, look, order};

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
