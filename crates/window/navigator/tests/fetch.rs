//! Downloads and updates as the Navigator runs them, shown with the stand-in language
//! server: a downloaded server stays as it is when the registry names a newer one until the
//! user asks for the update (NAV-09), and a server the user registered is started from the
//! user's file and never downloaded (LSP-04).
//!
//! Every address is a listener of the test's own on the loopback address, served through the
//! downloader's one code path; the thing downloaded is the stand-in, packed by the test.
//! Nothing leaves the machine.
//!
//! Every wait is for a state the Navigator tells, with a deadline; no test sleeps. A clause
//! that says nothing happened is read after a barrier ([`barrier`]): a hover, asked after the
//! fact in question, which the Navigator answers only after it handled everything asked
//! before it, whose answer carries the id of the ask and so cannot be taken for an earlier
//! one.

mod support;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use serde_json::Value;
use support::*;
use yaar_link::WorkspaceId;
use yaar_navigator::download::installed;
use yaar_navigator::{Ask, Offer, Platform, ServerState, State, Told, Tree, WorkspaceRoot};

/// Where the stand-in lies in the archives these tests pack: one folder above the program,
/// which `strip = 1` drops.
const IN_ARCHIVE: &str = "standin/standin-lsp";

/// The stand-in packed as a `tar.gz` at [`IN_ARCHIVE`], mode 0755, with a file that names
/// `version`, so that two versions are two different downloads.
fn archive_of(version: &str) -> Vec<u8> {
    let program = std::fs::read(standin()).expect("read the stand-in");
    tar_gz(&[
        (IN_ARCHIVE, &program, 0o755),
        ("standin/VERSION", version.as_bytes(), 0o644),
    ])
}

/// `text` as a TOML string.
fn q(text: impl AsRef<str>) -> String {
    serde_json::to_string(text.as_ref()).expect("a string")
}

/// The `[[standin.server.download]]` that pins `archive` as `version`, served at `url`,
/// for this machine.
fn download_of(version: &str, url: &str, archive: &[u8]) -> String {
    let platform = Platform::current().expect("a machine the built-in rows download for");
    format!(
        "[[standin.server.download]]\n\
         platform = {}\n\
         version = {}\n\
         url = {}\n\
         sha256 = \"{}\"\n\
         archive = \"tar.gz\"\n\
         strip = 1\n\
         program = \"standin-lsp\"\n\
         size = {}\n",
        q(platform.name()),
        q(version),
        q(url),
        sha256(archive),
        archive.len()
    )
}

/// The row `standin` whose one server is found nowhere on the machine (the scratch's `bin`
/// holds no stand-in) and has `downloads`; answers by `initialize` alone, logs to `log`.
fn row_with(log: &Path, downloads: &str) -> String {
    standin_row(
        "standin",
        &["--log", &log.display().to_string()],
        "answer",
        downloads,
    )
}

/// What the offer to download `version` of `archive` says.
fn offer_of(version: &str, archive: &[u8]) -> Offer {
    Offer {
        server: "standin".to_owned(),
        version: version.to_owned(),
        size: archive.len() as u64,
    }
}

/// A second row, `control`, for files ending in `.ctl` that the repository of the test holds
/// as `main.ctl`, whose one server `control-server` is found nowhere and downloads `archive`
/// as version 1 from `url`. The server's name differs from its row's, so that a download made
/// under the row's name instead of the server's installs where nothing looks for it. A test whose clause says no download happened asks for this row's
/// download after the fact, and reads the listener only once this one ended: a download that
/// the fact wrongly started was begun before this one, and has reached the listener long
/// before a whole download, its unpacking and its server's start are through.
fn control_row(url: &str, archive: &[u8]) -> String {
    control_row_named("control", "ctl", url, archive)
}

/// [`control_row`] under another name, for files `main.<extension>`: a test that reads its
/// clause twice, once for each of two Navigators on one servers folder, needs a control
/// that the first one did not leave installed.
fn control_row_named(name: &str, extension: &str, url: &str, archive: &[u8]) -> String {
    let platform = Platform::current().expect("a machine the built-in rows download for");
    format!(
        "[{name}]\n\
         extensions = {{ {extension} = \"standin\" }}\n\
         markers = [\"main.{extension}\"]\n\
         \n\
         [[{name}.server]]\n\
         name = \"{name}-server\"\n\
         trial = [\"--version\"]\n\
         \n\
         [[{name}.server.download]]\n\
         platform = {}\n\
         version = \"1\"\n\
         url = {}\n\
         sha256 = \"{}\"\n\
         archive = \"tar.gz\"\n\
         strip = 1\n\
         program = \"standin-lsp\"\n\
         size = {}\n",
        q(platform.name()),
        q(url),
        sha256(archive),
        archive.len()
    )
}

/// Asks for the download of the `control` row and waits until its server runs: the barrier
/// of a clause that says no download of another row began.
fn download_the_control(running: &mut Running, workspace: &WorkspaceId) {
    download_a_control(running, workspace, "control");
}

/// [`download_the_control`] for the control row `name`. The server that runs is the one
/// installed under the name of the row's server, not of the row.
fn download_a_control(running: &mut Running, workspace: &WorkspaceId, name: &str) {
    running.ask(Ask::Download {
        language: name.to_owned(),
    });
    let ready = running.wait_state(workspace, name, None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let program = ready.program.expect("a ready server has its program");
    assert!(
        program.ends_with(format!("servers/{name}-server/1/standin-lsp")),
        "the control server runs from where its server's name puts it, not its row's: {program:?}"
    );
}

/// The folder of the app's downloads of this test.
fn servers_of(scratch: &Scratch) -> PathBuf {
    scratch.root.join("servers")
}

/// Where the app keeps `version` of the stand-in's download for this test.
fn program_of(scratch: &Scratch, version: &str) -> PathBuf {
    scratch
        .root
        .join("servers")
        .join("standin")
        .join(version)
        .join("standin-lsp")
}

/// The place of `alpha` in the first line of `main.sil` of `repo`.
fn alpha(repo: &Repo, workspace: &WorkspaceId) -> yaar_navigator::Place {
    place(repo, workspace, Tree::Worktree, "main.sil", 1, "alpha", 0)
}

/// Asks a hover at [`alpha`] and waits for its answer. The Navigator handles one event at a
/// time and answers an ask after everything asked before it; the answer carries the id of the
/// ask, so that a list told earlier is never read for it. Whatever the Navigator told for the
/// asks before is in `running.told` when this returns.
fn barrier(running: &mut Running, repo: &Repo, workspace: &WorkspaceId) {
    running.hover(alpha(repo, workspace), DEADLINE);
}

/// The newest list of servers told so far, as `running` read it.
fn last_list(running: &Running) -> Vec<ServerState> {
    running
        .told
        .iter()
        .rev()
        .find_map(|told| match told {
            Told::Servers(list) => Some(list.clone()),
            _ => None,
        })
        .expect("a list of servers was told")
}

/// The state of the stand-in's server of `workspace` in the newest list.
fn state_of(running: &Running, workspace: &WorkspaceId) -> ServerState {
    last_list(running)
        .into_iter()
        .find(|state| state.workspace == *workspace && state.language == "standin")
        .expect("the stand-in's server is in the newest list")
}

/// Waits until one list of servers shows the stand-in's server of every workspace in
/// `workspaces` in a state `wanted` holds of. Waiting for each in turn would miss a state
/// told in the list that satisfied the one before.
fn wait_for_all(
    running: &mut Running,
    workspaces: &[&WorkspaceId],
    wanted: impl Fn(&ServerState) -> bool,
) {
    running.wait_told(
        "every workspace's stand-in server to reach the state",
        DEADLINE,
        |told| {
            let Told::Servers(list) = told else {
                return false;
            };
            workspaces.iter().all(|workspace| {
                list.iter().any(|state| {
                    state.workspace == **workspace && state.language == "standin" && wanted(state)
                })
            })
        },
    );
}

/// Whether a list of servers told in `told` shows a download of the stand-in's row on its
/// way (a control row's is another row's).
fn shows_downloading(told: &[Told]) -> bool {
    told.iter().any(|told| match told {
        Told::Servers(list) => list.iter().any(|state| {
            state.language == "standin" && matches!(state.state, State::Downloading { .. })
        }),
        _ => false,
    })
}

/// The request lines a [`Loopback`] heard.
fn lines(loopback: &Loopback) -> Vec<String> {
    loopback
        .heard()
        .into_iter()
        .map(|heard| heard.line)
        .collect()
}

/// Every line of a stand-in's log, as the JSON it is; none when the log is not there.
fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

/// A workspace of the repository `repo` for the Navigator.
fn root_of(repo: &Repo, workspace: &WorkspaceId, digit: char) -> WorkspaceRoot {
    WorkspaceRoot {
        id: workspace.clone(),
        repository: repository_id(digit),
        folder: Some(repo.root.clone()),
    }
}

// ---------------------------------------------------------------------------------------
// NAV-09: a downloaded server stays as it is until the user asks for the update.
// ---------------------------------------------------------------------------------------

#[test]
fn a_downloaded_server_stays_as_it_is_when_a_newer_one_is_named_until_the_user_asks_for_the_update()
{
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\n"),
            ("main.ctl", "\n"),
            ("main.ctlb", "\n"),
        ],
    );
    let (one, two) = (archive_of("1"), archive_of("2"));
    let loopback = Loopback::serve(vec![
        ("standin-1.tar.gz".to_owned(), one.clone()),
        ("standin-2.tar.gz".to_owned(), two.clone()),
        ("control.tar.gz".to_owned(), one.clone()),
        ("controlb.tar.gz".to_owned(), one.clone()),
    ]);
    let log = scratch.root.join("standin.log");
    // Each Navigator has a control row of its own, whose download is the barrier of what
    // the clauses say was not fetched: an update or a download that the Navigator wrongly
    // began by itself runs on a thread that no barrier of the Navigator's own orders.
    let pinning_one = row_with(
        &log,
        &download_of("1", &loopback.url("standin-1.tar.gz"), &one),
    ) + &control_row(&loopback.url("control.tar.gz"), &one);
    let pinning_two =
        row_with(
            &log,
            &download_of("2", &loopback.url("standin-2.tar.gz"), &two),
        ) + &control_row_named("controlb", "ctlb", &loopback.url("controlb.tar.gz"), &one);
    let servers = scratch.root.join("servers");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));

    // Navigator A: the row pins version 1, and no server is on the machine.
    let mut running = Running::start(scratch.config(&pinning_one));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state
            == State::Missing {
                offer: Some(offer_of("1", &one)),
            }
    });
    // Nothing is downloaded before the user's yes: only the control's request is heard.
    barrier(&mut running, &repo, &ws);
    download_the_control(&mut running, &ws);
    assert_eq!(lines(&loopback), ["GET /control.tar.gz"]);
    assert!(!shows_downloading(&running.told));
    assert_eq!(installed(&servers, "standin"), None);

    let asked_at = running.told.len();
    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    let downloading = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Downloading { .. })
    });
    assert_eq!(
        downloading.state,
        State::Downloading {
            offer: offer_of("1", &one)
        }
    );
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(program_of(&scratch, "1")));
    assert_eq!(ready.update, None);
    assert!(shows_downloading(&running.told[asked_at..]));
    let record = installed(&servers, "standin").expect("version 1 is installed");
    assert_eq!(record.version, "1");
    assert_eq!(record.sha256, sha256(&one));
    assert_eq!(
        lines(&loopback),
        ["GET /control.tar.gz", "GET /standin-1.tar.gz"]
    );
    running.end();

    // Navigator B, on the same folders: the row now pins version 2.
    let mut running = Running::start(scratch.config(&pinning_two));
    running.show(&repo, &ws, &repository);
    let before = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    // The server that runs is version 1, and the update is offered.
    assert_eq!(before.program, Some(program_of(&scratch, "1")));
    assert_eq!(before.update, Some(offer_of("2", &two)));
    barrier(&mut running, &repo, &ws);
    download_a_control(&mut running, &ws, "controlb");
    // It stays as it is: nothing is fetched, nothing changes of what runs or what is kept.
    let still = state_of(&running, &ws);
    assert_eq!(
        (&still.state, &still.program, still.pid, &still.update),
        (&State::Ready, &before.program, before.pid, &before.update)
    );
    assert!(!shows_downloading(&running.told));
    assert_eq!(
        lines(&loopback),
        [
            "GET /control.tar.gz",
            "GET /standin-1.tar.gz",
            "GET /controlb.tar.gz"
        ]
    );
    assert_eq!(installed(&servers, "standin").unwrap().version, "1");
    assert!(!servers.join("standin").join("2").exists());

    // Only the user's ask updates it.
    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    let updated = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.program == Some(program_of(&scratch, "2"))
    });
    assert_eq!(updated.update, None);
    assert_ne!(updated.pid, before.pid);
    assert_eq!(
        lines(&loopback),
        [
            "GET /control.tar.gz",
            "GET /standin-1.tar.gz",
            "GET /controlb.tar.gz",
            "GET /standin-2.tar.gz"
        ]
    );
    let record = installed(&servers, "standin").expect("version 2 is installed");
    assert_eq!(record.version, "2");
    assert_eq!(record.sha256, sha256(&two));
    // The older version is gone, and the one that ran is not.
    assert!(!servers.join("standin").join("1").exists());
    assert!(
        !is_alive(before.pid.expect("a process")),
        "the server of version 1 still runs"
    );
    running.end();
}

#[test]
fn a_download_asked_for_a_server_that_runs_with_no_update_to_show_does_nothing() {
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "project",
        &[("main.sil", "def alpha\n"), ("main.ctl", "\n")],
    );
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let archive = archive_of("1");
    let loopback = Loopback::serve(vec![("standin-1.tar.gz".to_owned(), archive.clone())]);
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("1", &loopback.url("standin-1.tar.gz"), &archive),
    ) + &control_row(&loopback.url("standin-1.tar.gz"), &archive);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.update, None);

    let asked_at = running.told.len();
    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    download_the_control(&mut running, &ws);
    // The one request is the control's.
    assert_eq!(lines(&loopback), ["GET /standin-1.tar.gz"]);
    assert!(!servers_of(&scratch).join("standin").exists());
    assert!(servers_of(&scratch).join("control-server").exists());
    let standin_states: Vec<State> = running.told[asked_at..]
        .iter()
        .filter_map(|told| match told {
            Told::Servers(list) => Some(list),
            _ => None,
        })
        .flatten()
        .filter(|state| state.language == "standin")
        .map(|state| state.state.clone())
        .collect();
    assert!(
        standin_states.iter().all(|state| *state == State::Ready),
        "{standin_states:#?}"
    );
    // Nothing restarted the server either.
    assert_eq!(state_of(&running, &ws).pid, ready.pid);
    running.end();
}

// ---------------------------------------------------------------------------------------
// LSP-04: a row of the user replaces a built-in row and is never downloaded.
// ---------------------------------------------------------------------------------------

/// The built-in row of the tests of LSP-04: titled, with a download for this machine, and
/// the stand-in found on the machine in the scratch's `bin`.
fn row_with_a_title(downloads: &str) -> String {
    format!(
        "[standin]\n\
         title = \"Stand-in tongue\"\n\
         extensions = {{ sil = \"standin\" }}\n\
         markers = [\"main.sil\"]\n\
         \n\
         [[standin.server]]\n\
         name = \"standin\"\n\
         machine = [\"standin-lsp\"]\n\
         trial = [\"--version\"]\n\
         {downloads}"
    )
}

/// The row `standin` as the Navigator told it at its start: its title and extensions.
fn standin_language(running: &Running) -> (String, Vec<String>) {
    let language = running
        .told
        .iter()
        .find_map(|told| match told {
            Told::Languages(languages) => {
                languages.iter().find(|language| language.name == "standin")
            }
            _ => None,
        })
        .expect("the Navigator tells the row standin at its start");
    (language.title.clone(), language.extensions.clone())
}

/// Writes the user's file: a row `standin` with one server `mine` that is `command` and
/// nothing else.
fn write_user_row(scratch: &Scratch, command: &[String]) {
    let command: Vec<String> = command.iter().map(q).collect();
    std::fs::write(
        scratch.config("").user_file,
        format!(
            "[[standin.server]]\n\
             name = \"mine\"\n\
             command = [{}]\n",
            command.join(", ")
        ),
    )
    .expect("write the user's file");
}

#[test]
fn a_row_of_the_user_replaces_a_built_in_row_is_started_from_the_registry_file_and_is_never_downloaded()
 {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let archive = archive_of("1");
    let loopback = Loopback::serve(vec![("standin-1.tar.gz".to_owned(), archive.clone())]);
    let downloads = download_of("1", &loopback.url("standin-1.tar.gz"), &archive);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mine = scratch.root.join("mine.log");

    // The one registry file of the user's, in the app's data folder.
    let config = scratch.config(&row_with_a_title(&downloads));
    assert_eq!(
        config.user_file,
        scratch.root.join("data").join(yaar_navigator::USER_FILE)
    );
    assert_eq!(yaar_navigator::USER_FILE, "language-servers.toml");

    // A server of the built-in row is on the machine, and would run if the row were used.
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    write_user_row(
        &scratch,
        &[
            standin().display().to_string(),
            "--log".to_owned(),
            mine.display().to_string(),
        ],
    );
    let mut running = Running::start(config);
    // The user's row names no title and no extensions: it takes the built-in row's, which
    // the Navigator tells at its start, before any file is shown.
    assert_eq!(
        standin_language(&running),
        ("Stand-in tongue".to_owned(), vec!["sil".to_owned()])
    );
    running.show(&repo, &ws, &repository);

    // "Replaces a built-in row": the user's server runs, not the one on the machine; the
    // title and the markers that started it are the built-in row's.
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert!(ready.registered);
    assert_eq!(ready.title, "Stand-in tongue");
    assert_eq!(ready.server.as_deref(), Some("mine"));
    assert_eq!(ready.program, Some(standin()));
    assert_eq!(ready.update, None);
    // "Is started from the one registry file": the server was started with the arguments
    // the file gives. The log line is written before the answer to `initialize`.
    let starts: Vec<Value> = log_of(&mine)
        .into_iter()
        .filter(|line| line["event"] == "start")
        .collect();
    assert_eq!(starts.len(), 1, "{starts:#?}");
    assert_eq!(
        starts[0]["args"],
        serde_json::json!(["--log", mine.display().to_string()])
    );
    running.end();
}

#[test]
fn a_registered_server_whose_command_fails_is_not_offered_and_a_download_asked_for_it_does_nothing()
{
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "project",
        &[("main.sil", "def alpha\n"), ("main.ctl", "\n")],
    );
    let archive = archive_of("1");
    let loopback = Loopback::serve(vec![
        ("standin-1.tar.gz".to_owned(), archive.clone()),
        ("control.tar.gz".to_owned(), archive.clone()),
    ]);
    let downloads = download_of("1", &loopback.url("standin-1.tar.gz"), &archive);
    let failing = scratch.put_failing(&scratch.root.join("tools"), "failing", 1);
    write_user_row(&scratch, &[failing.display().to_string()]);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    // The built-in row has a download, and its server is on the machine: the row of the
    // user replaces it whole, so that server is no candidate when the user's command fails.
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let builtin =
        row_with_a_title(&downloads) + &control_row(&loopback.url("control.tar.gz"), &archive);
    let mut running = Running::start(scratch.config(&builtin));
    assert_eq!(
        standin_language(&running),
        ("Stand-in tongue".to_owned(), vec!["sil".to_owned()])
    );
    running.show(&repo, &ws, &repository);

    // "When its command fails, no download is offered for it."
    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { .. })
    });
    assert_eq!(missing.state, State::Missing { offer: None });
    assert_eq!((missing.server, missing.pid), (None, None));
    assert!(missing.registered);
    assert!(!runs(&failing).is_empty(), "the command was never run");

    // "Is never downloaded": the ask to download, and the ask to update, change nothing.
    let asked_at = running.told.len();
    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    download_the_control(&mut running, &ws);
    // The one request is the control's, and the registered row never showed a download.
    assert_eq!(lines(&loopback), ["GET /control.tar.gz"]);
    let standin_states: Vec<State> = running.told[asked_at..]
        .iter()
        .filter_map(|told| match told {
            Told::Servers(list) => Some(list),
            _ => None,
        })
        .flatten()
        .filter(|state| state.language == "standin")
        .map(|state| state.state.clone())
        .collect();
    assert!(
        standin_states
            .iter()
            .all(|state| *state == State::Missing { offer: None }),
        "{standin_states:#?}"
    );
    assert!(!servers_of(&scratch).join("standin").exists());
    // The server of the built-in row on the machine never ran, not before the control ended
    // either: no list told at all shows the row ready, or on a server or program that is not
    // the user's command.
    let ever: Vec<ServerState> = running
        .told
        .iter()
        .filter_map(|told| match told {
            Told::Servers(list) => Some(list),
            _ => None,
        })
        .flatten()
        .filter(|state| state.language == "standin")
        .cloned()
        .collect();
    assert!(
        ever.iter().all(|state| state.state != State::Ready
            && state
                .server
                .as_deref()
                .is_none_or(|server| server == "mine")
            && state
                .program
                .as_ref()
                .is_none_or(|program| *program == failing)),
        "{ever:#?}"
    );
    running.end();
}

// ---------------------------------------------------------------------------------------
// What a download does while it is on its way, and when it fails.
// ---------------------------------------------------------------------------------------

/// A listener of the test's own on the loopback address that serves one file and holds each
/// request, once heard, until the test lets it go: a download that is on its way for as long
/// as the test wants it to be. Its threads end when the process does.
struct Held {
    port: u16,
    open: Arc<(Mutex<bool>, Condvar)>,
    heard: Arc<Mutex<Vec<String>>>,
    arrived: Receiver<()>,
}

impl Held {
    /// A listener that answers `GET /<name>` with `bytes` once released.
    fn serve(name: &str, bytes: Vec<u8>) -> Held {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen on the loopback address");
        let port = listener.local_addr().expect("its address").port();
        let open = Arc::new((Mutex::new(false), Condvar::new()));
        let heard = Arc::new(Mutex::new(Vec::new()));
        let (arrived_at, arrived) = mpsc::channel();
        let (kept, gate) = (Arc::clone(&heard), Arc::clone(&open));
        let name = format!("GET /{name}");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (kept, gate, bytes, name) = (
                    Arc::clone(&kept),
                    Arc::clone(&gate),
                    bytes.clone(),
                    name.clone(),
                );
                let arrived_at = arrived_at.clone();
                std::thread::spawn(move || {
                    Held::answer(&stream, &kept, &gate, &arrived_at, &name, &bytes);
                });
            }
        });
        Held {
            port,
            open,
            heard,
            arrived,
        }
    }

    /// Reads one request, notes it, tells the test it came, waits for the gate to open, and
    /// answers.
    fn answer(
        stream: &TcpStream,
        heard: &Mutex<Vec<String>>,
        gate: &(Mutex<bool>, Condvar),
        arrived: &mpsc::Sender<()>,
        name: &str,
        bytes: &[u8],
    ) {
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        if !matches!(reader.read_line(&mut request), Ok(read) if read > 0) {
            return;
        }
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(read) if read > 0 && line.trim_end() != "" => {}
                _ => break,
            }
        }
        let request = request
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let _ = arrived.send(());
        let (lock, opened) = gate;
        let mut open = lock.lock().unwrap_or_else(PoisonError::into_inner);
        while !*open {
            open = opened.wait(open).unwrap_or_else(PoisonError::into_inner);
        }
        drop(open);
        let response = if request == name {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .into_bytes();
            response.extend_from_slice(bytes);
            response
        } else {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        };
        let mut stream = stream;
        let _ = stream.write_all(&response);
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
    }

    /// `http://127.0.0.1:<port>/<name>`.
    fn url(&self, name: &str) -> String {
        format!("http://127.0.0.1:{}/{name}", self.port)
    }

    /// Waits until a request has arrived that was not waited for before.
    fn wait_for_a_request(&self) {
        self.arrived
            .recv_timeout(DEADLINE)
            .expect("no request reached the listener in time");
    }

    /// Every request heard, in order.
    fn heard(&self) -> Vec<String> {
        self.heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Lets every request, heard or still to come, be answered.
    fn release(&self) {
        let (lock, opened) = &*self.open;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        opened.notify_all();
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.release();
    }
}

/// Installs version `version` of the stand-in the way the downloader leaves it.
fn install(scratch: &Scratch, version: &str) {
    let folder = scratch.root.join("servers").join("standin");
    scratch.put_standin(&folder.join(version), "standin-lsp");
    std::fs::write(
        folder.join("installed.toml"),
        format!(
            "version = {}\nsha256 = \"{}\"\nprogram = \"standin-lsp\"\n",
            q(version),
            "b".repeat(64)
        ),
    )
    .expect("write installed.toml");
}

#[test]
fn an_update_leaves_the_running_server_serving_until_its_download_ended() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    install(&scratch, "1");
    let two = archive_of("2");
    let held = Held::serve("standin-2.tar.gz", two.clone());
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("2", &held.url("standin-2.tar.gz"), &two),
    );
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let before = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.update.is_some()
    });

    let asked_at = running.told.len();
    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    held.wait_for_a_request();
    // The download is on its way. The server of version 1 runs as it did, shows what it
    // showed, and still answers; nothing says "downloading".
    let answer = running.hover(alpha(&repo, &ws), DEADLINE);
    assert!(!answer.guessed, "{answer:#?}");
    assert_eq!(answer.value.as_deref(), Some("standin: alpha"));
    let meanwhile = state_of(&running, &ws);
    assert_eq!(
        (
            &meanwhile.state,
            &meanwhile.program,
            meanwhile.pid,
            &meanwhile.update
        ),
        (&State::Ready, &before.program, before.pid, &before.update)
    );
    assert!(!shows_downloading(&running.told[asked_at..]));
    assert!(program_of(&scratch, "1").exists());

    // An update asked for again while it is on its way is not a second download. What a
    // second download would send comes from a thread that has no barrier of its own, so the
    // listener's list is read once more at the end, after the Navigator told both versions.
    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    barrier(&mut running, &repo, &ws);

    held.release();
    let after = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.program == Some(program_of(&scratch, "2"))
    });
    assert_eq!(after.update, None);
    assert_ne!(after.pid, before.pid);
    // The old server was stopped before the new one's state was told.
    assert!(!is_alive(before.pid.expect("a process")));
    assert_eq!(held.heard(), ["GET /standin-2.tar.gz"]);
    running.end();
}

#[test]
fn a_workspace_shown_while_an_update_is_on_its_way_runs_the_new_version_when_it_ends() {
    let scratch = Scratch::new();
    let first = scratch.repository("first", &[("main.sil", "def alpha\n")]);
    let second = scratch.repository("second", &[("main.sil", "def alpha\n")]);
    install(&scratch, "1");
    let two = archive_of("2");
    let held = Held::serve("standin-2.tar.gz", two.clone());
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("2", &held.url("standin-2.tar.gz"), &two),
    );
    let (ws1, ws2) = (workspace_id('1'), workspace_id('2'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&first, &ws1, &repository_id('1'));
    let before_first = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.update.is_some()
    });

    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    held.wait_for_a_request();
    // A second workspace is shown while the update is on its way. It was not part of the
    // update that began, and finds version 1 installed: it runs it, and offers the update.
    running.ask(Ask::Workspaces(vec![
        root_of(&first, &ws1, '1'),
        root_of(&second, &ws2, '2'),
    ]));
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let before_second = running.wait_state(&ws2, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(before_second.program, Some(program_of(&scratch, "1")));
    assert_eq!(before_second.update, Some(offer_of("2", &two)));

    held.release();
    // The update ended, and removed version 1: both workspaces run version 2, and offer
    // nothing, and the server of the second that ran version 1 is gone.
    let program = Some(program_of(&scratch, "2"));
    wait_for_all(&mut running, &[&ws1, &ws2], |state| {
        state.state == State::Ready && state.program == program && state.update.is_none()
    });
    assert!(!program_of(&scratch, "1").exists());
    for before in [&before_first, &before_second] {
        assert!(
            !is_alive(before.pid.expect("a process")),
            "a server of version 1 still runs"
        );
    }
    assert_eq!(held.heard(), ["GET /standin-2.tar.gz"]);
    running.end();
}

#[test]
fn an_update_that_ends_well_starts_again_a_key_still_starting_on_the_older_version() {
    let scratch = Scratch::new();
    let first = scratch.repository("first", &[("main.sil", "def alpha\n")]);
    let second = scratch.repository("second", &[("main.sil", "def alpha\n")]);
    install(&scratch, "1");
    let two = archive_of("2");
    let held = Held::serve("standin-2.tar.gz", two.clone());
    // The server is ready once `{root}/go` exists: the second repository holds it, the first
    // does not, so that its server stays starting until the test makes the file.
    let row = standin_row(
        "standin",
        &[
            "--ready",
            "notification",
            "--ready-when",
            "{root}/go",
            "--log",
            &scratch.root.join("standin.log").display().to_string(),
        ],
        "notification",
        &download_of("2", &held.url("standin-2.tar.gz"), &two),
    );
    std::fs::write(second.root.join("go"), "").expect("make the file the second waits for");
    let (ws1, ws2) = (workspace_id('1'), workspace_id('2'));
    let mut running = Running::start(scratch.config(&row));
    running.ask(Ask::Workspaces(vec![
        root_of(&first, &ws1, '1'),
        root_of(&second, &ws2, '2'),
    ]));
    // The second workspace's server is ready on version 1 and shows the update; the first
    // workspace is shown after it, and its server starts on version 1 and does not get ready.
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let ready = running.wait_state(&ws2, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.update.is_some()
    });
    running.ask(Ask::OnScreen(Some(ws1.clone())));
    let starting = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Starting && state.pid.is_some()
    });
    assert_eq!(starting.program, Some(program_of(&scratch, "1")));

    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    held.wait_for_a_request();
    held.release();
    // The update ended and removed version 1. The key that was ready on it runs version 2;
    // so does the key that was starting on it, which is let go ready only now: it is not the
    // process of version 1 that gets ready.
    running.wait_state(&ws2, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.program == Some(program_of(&scratch, "2"))
    });
    std::fs::write(first.root.join("go"), "").expect("make the file the first waits for");
    let after = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(after.program, Some(program_of(&scratch, "2")));
    assert_eq!(after.update, None);
    assert_ne!(after.pid, starting.pid);
    assert!(!program_of(&scratch, "1").exists());
    for before in [ready.pid, starting.pid] {
        assert!(
            !is_alive(before.expect("a process")),
            "a server of version 1 still runs"
        );
    }
    running.end();
}

#[test]
fn an_update_that_fails_keeps_the_version_that_ran_and_still_offers_the_newer_one() {
    let scratch = Scratch::new();
    let repo = scratch.repository(
        "project",
        &[("main.sil", "def alpha\n"), ("main.ctl", "\n")],
    );
    install(&scratch, "1");
    let two = archive_of("2");
    // The address serves bytes that are not the ones pinned.
    let loopback = Loopback::serve(vec![
        ("standin-2.tar.gz".to_owned(), archive_of("other")),
        ("control.tar.gz".to_owned(), archive_of("1")),
    ]);
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("2", &loopback.url("standin-2.tar.gz"), &two),
    ) + &control_row(&loopback.url("control.tar.gz"), &archive_of("1"));
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    let before = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.update.is_some()
    });

    running.ask(Ask::Update {
        language: "standin".to_owned(),
    });
    // The barrier: the control row's whole download, asked after the update, ends after the
    // failed one (which is a digest that does not match) was handled and the Navigator told
    // what it did about it.
    download_the_control(&mut running, &ws);
    let mut heard = lines(&loopback);
    heard.sort();
    assert_eq!(heard, ["GET /control.tar.gz", "GET /standin-2.tar.gz"]);

    // The server that ran runs still: the same process, ready, the update still offered.
    let after = state_of(&running, &ws);
    assert_eq!(
        (&after.state, &after.program, after.pid, &after.update),
        (
            &State::Ready,
            &before.program,
            before.pid,
            &Some(offer_of("2", &two))
        )
    );
    assert!(is_alive(before.pid.expect("a process")));
    assert!(!shows_downloading(&running.told));
    assert_eq!(
        installed(&servers_of(&scratch), "standin").unwrap().version,
        "1"
    );
    assert!(!servers_of(&scratch).join("standin").join("2").exists());
    running.end();
}

#[test]
fn a_download_on_its_way_is_not_started_twice_and_a_workspace_shown_meanwhile_is_served_when_it_ends()
 {
    let scratch = Scratch::new();
    let first = scratch.repository("first", &[("main.sil", "def alpha\n")]);
    let second = scratch.repository("second", &[("main.sil", "def alpha\n")]);
    let one = archive_of("1");
    let held = Held::serve("standin-1.tar.gz", one.clone());
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("1", &held.url("standin-1.tar.gz"), &one),
    );
    let (ws1, ws2) = (workspace_id('1'), workspace_id('2'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&first, &ws1, &repository_id('1'));
    running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { offer: Some(_) })
    });

    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Downloading { .. })
    });
    held.wait_for_a_request();
    // A second workspace is shown while the download is on its way. It was not part of the
    // download that began: it is missing, with the offer, as it was found.
    running.ask(Ask::Workspaces(vec![
        root_of(&first, &ws1, '1'),
        root_of(&second, &ws2, '2'),
    ]));
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let late = running.wait_state(&ws2, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { .. })
    });
    assert_eq!(
        late.state,
        State::Missing {
            offer: Some(offer_of("1", &one))
        }
    );
    // The user asks again: the row's download is on its way, and this one does nothing, even
    // for a key that offers it.
    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    barrier(&mut running, &second, &ws2);
    assert_eq!(state_of(&running, &ws2).state, late.state);
    // Still one request, read after the barrier of the second ask. A thread that a second
    // download would have started has no barrier of its own: the list is read again below.
    assert_eq!(held.heard(), ["GET /standin-1.tar.gz"]);

    held.release();
    // The one download ended: both workspaces find the server it installed.
    let program = Some(program_of(&scratch, "1"));
    wait_for_all(&mut running, &[&ws1, &ws2], |state| {
        state.state == State::Ready && state.program == program
    });
    barrier(&mut running, &second, &ws2);
    assert_eq!(held.heard(), ["GET /standin-1.tar.gz"]);
    running.end();
}

#[test]
fn a_workspace_that_leaves_while_its_download_is_on_its_way_is_not_searched_again() {
    let scratch = Scratch::new();
    let first = scratch.repository("first", &[("main.sil", "def alpha\n")]);
    let second = scratch.repository("second", &[("main.sil", "def alpha\n")]);
    let one = archive_of("1");
    let held = Held::serve("standin-1.tar.gz", one.clone());
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("1", &held.url("standin-1.tar.gz"), &one),
    );
    let (ws1, ws2) = (workspace_id('1'), workspace_id('2'));
    let mut running = Running::start(scratch.config(&row));
    running.ask(Ask::Workspaces(vec![
        root_of(&first, &ws1, '1'),
        root_of(&second, &ws2, '2'),
    ]));
    for ws in [&ws2, &ws1] {
        running.ask(Ask::OnScreen(Some(ws.clone())));
        running.wait_state(ws, "standin", None, DEADLINE, |state| {
            matches!(state.state, State::Missing { offer: Some(_) })
        });
    }

    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    wait_for_all(&mut running, &[&ws1, &ws2], |state| {
        matches!(state.state, State::Downloading { .. })
    });
    held.wait_for_a_request();
    // The second workspace leaves the list; its server is forgotten.
    running.ask(Ask::Workspaces(vec![root_of(&first, &ws1, '1')]));
    barrier(&mut running, &first, &ws1);
    assert!(
        last_list(&running)
            .iter()
            .all(|state| state.workspace != ws2),
        "{:#?}",
        last_list(&running)
    );

    held.release();
    running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.program == Some(program_of(&scratch, "1"))
    });
    barrier(&mut running, &first, &ws1);
    // The workspace that left was not searched again: it is not in what is told now.
    assert!(
        last_list(&running)
            .iter()
            .all(|state| state.workspace != ws2),
        "{:#?}",
        last_list(&running)
    );
    running.end();
}

#[test]
fn a_download_that_fails_shows_the_offer_again_installs_nothing_and_can_be_asked_for_again() {
    let scratch = Scratch::new();
    let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
    let one = archive_of("1");
    // The address the row names serves bytes that are not the ones pinned.
    let loopback = Loopback::serve(vec![
        ("bad.tar.gz".to_owned(), archive_of("not the one")),
        ("good.tar.gz".to_owned(), one.clone()),
    ]);
    let row = row_with(
        &scratch.root.join("standin.log"),
        &download_of("1", &loopback.url("bad.tar.gz"), &one),
    );
    let servers = scratch.root.join("servers");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { offer: Some(_) })
    });

    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Downloading { .. })
    });
    // The download failed: the offer is back, and nothing is installed.
    let missing = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        matches!(state.state, State::Missing { .. })
    });
    assert_eq!(
        missing.state,
        State::Missing {
            offer: Some(offer_of("1", &one))
        }
    );
    assert_eq!(installed(&servers, "standin"), None);
    assert_eq!(lines(&loopback), ["GET /bad.tar.gz"]);

    // The user may ask again, and now the address serves the right bytes.
    loopback.redirect("bad.tar.gz", &loopback.url("good.tar.gz"));
    running.ask(Ask::Download {
        language: "standin".to_owned(),
    });
    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(ready.program, Some(program_of(&scratch, "1")));
    assert_eq!(installed(&servers, "standin").unwrap().sha256, sha256(&one));
    running.end();
}

#[test]
fn the_reason_a_download_failed_is_one_line_on_the_standard_error() {
    let this_test = "the_reason_a_download_failed_is_one_line_on_the_standard_error";
    if std::env::var_os("YAAR_FETCH_CHILD").is_some() {
        // The child process: a download whose bytes are not the ones pinned, ended when the
        // offer is back.
        let scratch = Scratch::new();
        let repo = scratch.repository("project", &[("main.sil", "def alpha\n")]);
        let pinned = archive_of("1");
        let loopback = Loopback::serve(vec![("bad.tar.gz".to_owned(), archive_of("other"))]);
        let row = row_with(
            &scratch.root.join("standin.log"),
            &download_of("1", &loopback.url("bad.tar.gz"), &pinned),
        );
        let (ws, repository) = (workspace_id('1'), repository_id('1'));
        let mut running = Running::start(scratch.config(&row));
        running.show(&repo, &ws, &repository);
        running.wait_state(&ws, "standin", None, DEADLINE, |state| {
            matches!(state.state, State::Missing { offer: Some(_) })
        });
        running.ask(Ask::Download {
            language: "standin".to_owned(),
        });
        running.wait_state(&ws, "standin", None, DEADLINE, |state| {
            matches!(state.state, State::Downloading { .. })
        });
        // The line is written before the key is searched again and the offer told.
        running.wait_state(&ws, "standin", None, DEADLINE, |state| {
            matches!(state.state, State::Missing { .. })
        });
        running.end();
        return;
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", this_test, "--nocapture", "--test-threads=1"])
        .env("YAAR_FETCH_CHILD", "1")
        .output()
        .expect("run the test in a process of its own");
    let said = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{said}");
    assert!(said.contains("1 passed"), "the child test ran: {said}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ours: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("yaar-navigator:"))
        .collect();
    assert_eq!(ours.len(), 1, "{stderr}");
    // One sentence naming the row and saying the bytes are not the ones pinned: too many of
    // them, or another digest.
    assert!(ours[0].contains("standin"), "{}", ours[0]);
    assert!(ours[0].contains("pinned"), "{}", ours[0]);
}
