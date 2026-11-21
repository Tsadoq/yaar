//! What the component tests of the composed Window share: the Window opened as the program
//! opens it, in a world of its own with its real Keeper behind a tap, and the few things
//! a user does that every test needs (a click into a pane, a folder opened, a key pressed).
//!
//! A test file starts with `mod common;` and takes what it needs.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyView, Entity, KeystrokeEvent, Modifiers, Subscription, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_frame::{BuildPane, Control, Frame, LinkStatus, Models, Opened, Pane, Wanted};
use yaar_link::{
    ReviewId, SessionId, SessionState, TerminalId, TerminalSize, ThreadId, WorkspaceId,
};
use yaar_navigator::{NavigatorConfig, STOP_AFTER, SystemClock};
use yaar_review_pane::ReviewPane;
use yaar_system_tests::standin::{Script, Step};
use yaar_system_tests::{Repo, Tap, World, descendants_of, is_alive, wait_until as wait_for};
use yaar_terminal_pane::TerminalPane;

pub use yaar_system_tests::DEADLINE;

/// How a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

/// The composed Window of one test, in a world of its own, with a tap on its link.
pub struct Composed {
    // Fields are dropped in this order. The programs the Keeper started are listed while it
    // still runs, the tap goes before the world it leads into, and once the world has
    // ended the Keeper the test waits until the programs are gone too.
    _listing: ListingPrograms,
    pub tap: Tap,
    pub world: World,
    pub frame: Entity<Frame>,
    pub models: Models,
    /// The review pane, as the test reads it.
    pub review: Entity<ReviewPane>,
    /// The terminal pane, as the test reads it.
    pub terminals: Entity<TerminalPane>,
    /// The actions the keystrokes ran since the list was last taken, by name, as the
    /// test platform tells them. A key the focused element took itself runs none.
    pub ran: Rc<RefCell<Vec<&'static str>>>,
    _watching_the_keys: Subscription,
    /// The repositories the test made, each opened as a workspace.
    pub repositories: Vec<Repo>,
    /// The agent the walk of the reviews started, once it did.
    pub agent: Option<RunningAgent>,
    /// The session a restart cut off, when the test made one before it opened the Window.
    pub cut_off: Option<CutOff>,
    /// Counts what the test makes, for names of its own.
    made: u32,
    _waiting: WaitingForPrograms,
}

/// An agent a test started in a terminal of the workspace on screen, and what the walk of
/// the reviews holds of it.
pub struct RunningAgent {
    /// The terminal it runs in.
    pub terminal: TerminalId,
    /// Its session.
    pub session: SessionId,
    /// The review of that session, which the review pane shows.
    pub review: ReviewId,
    /// The thread the walk wrote, once it wrote one.
    pub thread: Option<ThreadId>,
}

/// The processes the Keeper started, listed while it still runs: dropping this reads them
/// off the process table, which says no more once the Keeper is gone and its children
/// belong to the system.
struct ListingPrograms {
    keeper: u32,
    into: Rc<RefCell<Vec<u32>>>,
}

impl Drop for ListingPrograms {
    fn drop(&mut self) {
        *self.into.borrow_mut() = descendants_of(self.keeper);
    }
}

/// Waits until every process the Keeper started has ended. The world ends the Keeper
/// alone, and its terminals' programs end after it, when the closed terminal hangs up on
/// them: a test that started a process waits until it is gone.
struct WaitingForPrograms {
    started: Rc<RefCell<Vec<u32>>>,
}

impl Drop for WaitingForPrograms {
    fn drop(&mut self) {
        let started = self.started.borrow().clone();
        let gone = wait_for(DEADLINE, || started.iter().all(|pid| !is_alive(*pid)));
        // A test that already failed says why; a second panic in its unwinding would end
        // the process before it did.
        if !gone && !std::thread::panicking() {
            let left: Vec<u32> = started.into_iter().filter(|pid| is_alive(*pid)).collect();
            panic!("the programs of the Keeper did not end with it: {left:?}");
        }
    }
}

/// Opens the Window as `yaar_window::open` does: `yaar_window::init`, then the Frame with
/// one builder for each entry of `yaar_window::panes()`, and after them `extra`. The
/// review pane's and the terminal pane's builders are wrapped to keep the view the test
/// reads; nothing else of the composition differs. The Keeper is the built program of a
/// new world, reached through a tap, and the call returns once it has answered.
pub fn open_composed(
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    world.start_keeper();
    open_composed_in(world, cx, extra)
}

/// [`open_composed`] in a world the test made and started a Keeper in, which holds what
/// the test put there: a Window started against a Keeper that already holds state.
pub fn open_composed_in(
    world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
) -> (Composed, VisualTestContext) {
    open_composed_with(world, cx, extra, true, None)
}

/// [`open_composed_in`] for a second Window of the same test, after the Keeper of the world
/// was started again: the app was prepared by the first, and its keys are not bound twice.
pub fn open_another_composed_in(
    world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
) -> (Composed, VisualTestContext) {
    open_composed_with(world, cx, extra, false, None)
}

/// [`open_composed_in`] with a Navigator started as the program starts one, on
/// `navigator`, which [`navigator_for`] makes for the world: the composed Window with the
/// real Keeper and the real Navigator, and the stand-in language server beneath. The test
/// ends the Navigator with [`Composed::end_the_navigator`].
pub fn open_composed_with_a_navigator(
    world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
    navigator: NavigatorConfig,
) -> (Composed, VisualTestContext) {
    open_composed_with(world, cx, extra, true, Some(navigator))
}

/// How the Navigator of a test finds the stand-in language server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// Installed on the machine: a copy of the stand-in named `standin-lsp` in a folder of
    /// the world, which is the Navigator's search path.
    OnTheMachine,
    /// The project's own: the file `tools/standin-lsp` of the repository, which the test
    /// puts there ([`put_the_standin_in`]). A server inside a repository runs only after the
    /// user said yes.
    InTheProject,
}

/// The folder of the world that holds what the Navigator of a test keeps: the downloads, the
/// user's registry (none), the copy of the stand-in.
fn navigator_data(world: &World) -> PathBuf {
    world.home().with_file_name("navigator-data")
}

/// The Navigator's cache folder in a world: where the Keeper, whose home is the world's,
/// finds the app's cache, `yaar_link::cache_dir()` of its environment, and the Navigator's
/// folder in it. The Keeper writes an export only there.
pub fn navigator_cache(world: &World) -> PathBuf {
    let cache = if cfg!(target_os = "macos") {
        world.home().join("Library/Caches")
    } else {
        world.home().join(".cache")
    };
    cache
        .join(yaar_link::APP_FOLDER)
        .join(yaar_navigator::CACHE_FOLDER)
}

/// The registry of a test's Navigator: one row, `standin`, for the invented language whose
/// files end in `.sil` and for `extra_extensions` (each mapped to the same language id),
/// started when the worktree holds `marker`, with one server, the stand-in, found as `found`
/// says.
pub fn standin_row(found: Found, marker: &str, extra_extensions: &[&str]) -> String {
    standin_row_started_with(found, marker, extra_extensions, &[])
}

/// [`standin_row`] with the stand-in started with `args`, each of which may name a variable
/// of the row (`{root}`).
pub fn standin_row_started_with(
    found: Found,
    marker: &str,
    extra_extensions: &[&str],
    args: &[&str],
) -> String {
    let args: Vec<String> = args
        .iter()
        .map(|arg| format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    let mut extensions = String::from("sil = \"standin\"");
    for extension in extra_extensions {
        extensions.push_str(&format!(", {extension} = \"standin\""));
    }
    let find = match found {
        Found::OnTheMachine => "machine = [\"standin-lsp\"]",
        Found::InTheProject => "project = [\"tools/standin-lsp\"]",
    };
    format!(
        "[standin]\n\
         title = \"Stand-in\"\n\
         extensions = {{ {extensions} }}\n\
         markers = [\"{marker}\"]\n\
         \n\
         [[standin.server]]\n\
         name = \"standin\"\n\
         {find}\n\
         args = [{}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}\n",
        args.join(", ")
    )
}

/// The configuration of the Navigator of a test in `world`: the registry `row`, every folder
/// in the world, the Navigator's cache where the Keeper of the world takes its exports
/// ([`navigator_cache`]), the stand-in on the search path when it is installed on the
/// machine, a short search path and environment of the world's own for the servers, and no
/// download: nothing of the user's is read, and nothing leaves the machine.
pub fn navigator_for(world: &World, found: Found, row: &str) -> NavigatorConfig {
    let data = navigator_data(world);
    let folder = data.join("machine");
    std::fs::create_dir_all(&folder).expect("make the folder of the machine's programs");
    let mut search_path = Vec::new();
    if found == Found::OnTheMachine {
        // A second window of the test finds the copy there, which the first may run.
        let copy = folder.join("standin-lsp");
        if !copy.exists() {
            std::fs::copy(yaar_system_tests::program("standin-lsp"), &copy)
                .expect("copy the stand-in language server");
        }
        search_path.push(folder);
    }
    let tmp = world.home().with_file_name("tmp");
    let environment = vec![
        ("HOME".into(), world.home().into_os_string()),
        ("TMPDIR".into(), tmp.into_os_string()),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
    ];
    NavigatorConfig {
        builtin: row.to_owned(),
        user_file: data.join(yaar_navigator::USER_FILE),
        servers: data.join(yaar_navigator::SERVERS_FOLDER),
        cache: navigator_cache(world),
        search_path,
        environment,
        platform: None,
        stop_after: STOP_AFTER,
        clock: std::sync::Arc::new(SystemClock),
    }
}

/// Puts a copy of the stand-in language server in the repository as the project's own,
/// `tools/standin-lsp`, and keeps it out of what git sees (the exclude file of the
/// repository, which no change and no commit holds): the worktree has the file, and the
/// review has no row for it.
pub fn put_the_standin_in(repository: &Repo) {
    let program = repository.path().join("tools/standin-lsp");
    std::fs::create_dir_all(program.parent().expect("a folder above")).expect("make tools");
    std::fs::copy(yaar_system_tests::program("standin-lsp"), &program)
        .expect("copy the stand-in language server");
    let exclude = repository.path().join(".git/info/exclude");
    let mut held = std::fs::read_to_string(&exclude).unwrap_or_default();
    held.push_str("/tools/\n");
    std::fs::write(&exclude, held).expect("write the exclude file");
}

fn open_composed_with(
    mut world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
    prepare_the_app: bool,
    navigator: Option<NavigatorConfig>,
) -> (Composed, VisualTestContext) {
    let keeper = world.keeper_pid().expect("the world's Keeper runs");
    let tap = world.tap();

    // A real socket and a real process: the connection is read by a thread, and answered
    // by a program, that the test executor cannot see.
    cx.executor().allow_parking();
    let ran: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let watching = cx.update(|cx| {
        if prepare_the_app {
            yaar_window::init(cx);
        }
        let ran = ran.clone();
        cx.observe_keystrokes(move |event: &KeystrokeEvent, _, _| {
            if let Some(action) = &event.action {
                ran.borrow_mut().push(action.name());
            }
        })
    });

    let kept: Rc<RefCell<Vec<(&'static str, AnyView)>>> = Rc::default();
    let mut builders: Vec<BuildPane> = yaar_window::panes()
        .into_iter()
        .map(|entry| {
            let build = (entry.build)();
            let kept = kept.clone();
            let wrapped: BuildPane = Box::new(move |models, window, cx| {
                let pane: Pane = build(models, window, cx);
                kept.borrow_mut().push((pane.name, pane.view.clone()));
                pane
            });
            wrapped
        })
        .collect();
    builders.extend(extra);
    let Opened {
        window,
        frame,
        models,
    } = cx
        .update(|cx| match navigator {
            // As the program opens its window.
            Some(navigator) => yaar_window::open_panes(tap.socket(), builders, navigator, cx),
            None => yaar_frame::open(tap.socket(), yaar_window::VERSION, builders, cx),
        })
        .expect("open the composed Window on the test platform");
    let mut cx = VisualTestContext::from_window(window, cx);
    // The window a user types in is the active one; GPUI tells no focus change to a window
    // that is not, and the terminal pane hands the focus on by one.
    cx.update(|window, _| window.activate_window());

    wait_until(
        &mut cx,
        "the Keeper to answer through the tap",
        |cx| match models.link.read_with(cx, |link, _| link.status().clone()) {
            LinkStatus::Starting => false,
            LinkStatus::Connected(answered) => {
                assert_eq!(answered.pid, keeper, "the Keeper of the world answered");
                true
            }
            other => panic!("the Window did not reach the Keeper of the world: {other:?}"),
        },
    );

    let view = |name: &str| {
        kept.borrow()
            .iter()
            .find(|(of, _)| *of == name)
            .map(|(_, view)| view.clone())
            .unwrap_or_else(|| panic!("the Window has a pane {name}"))
    };
    let review = view(yaar_review_pane::NAME)
        .downcast::<ReviewPane>()
        .unwrap_or_else(|_| panic!("the review pane builds a ReviewPane"));
    let terminals = view(yaar_terminal_pane::NAME)
        .downcast::<TerminalPane>()
        .unwrap_or_else(|_| panic!("the terminal pane builds a TerminalPane"));

    let started: Rc<RefCell<Vec<u32>>> = Rc::default();
    (
        Composed {
            _listing: ListingPrograms {
                keeper,
                into: started.clone(),
            },
            tap,
            world,
            frame,
            models,
            review,
            terminals,
            ran,
            _watching_the_keys: watching,
            repositories: Vec::new(),
            agent: None,
            cut_off: None,
            made: 0,
            _waiting: WaitingForPrograms { started },
        },
        cx,
    )
}

/// Gives the repository the identity of a user, in its own configuration. The Keeper commits
/// with whatever identity git finds, and in a world git reads no global configuration: where
/// git guesses one from the machine's names (macOS) a commit goes through, and where it does
/// not (Linux) git refuses it.
pub fn give_an_identity(repository: &Repo) {
    repository.git(&["config", "user.name", "A Test"]);
    repository.git(&["config", "user.email", "test@example.invalid"]);
}

/// A new repository of `world` with something to review: on a branch that left `main` with
/// one commit, and with one more change that is not committed.
pub fn repository_to_review(world: &World, name: &str) -> Repo {
    let repository = world.repository(name);
    give_an_identity(&repository);
    repository.git(&["checkout", "--quiet", "-b", "work"]);
    repository.write("notes.txt", "one\ntwo\nthree\n");
    repository.commit("Add the notes");
    repository.write("notes.txt", "one\n2\nthree\n");
    repository
}

/// What [`a_session_cut_off`] leaves: the workspace, the session of Claude Code a restart cut
/// off, and the id that CLI gave the session.
pub struct CutOff {
    /// The repository of the workspace, opened in the Keeper.
    pub repo: Repo,
    /// The workspace the session worked in.
    pub workspace: WorkspaceId,
    /// The session, listed resumable by the Keeper that started after the kill.
    pub session: SessionId,
    /// The id the CLI gave its session, which its resume command names.
    pub cli_session: String,
    /// The file that, once made, lets a stand-in that follows the world's `default` script
    /// end. Nobody makes it before the test does.
    pub gate: PathBuf,
}

/// The size of the terminals a client of the world opens.
const CLIENT_SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// Brings `world` to the state a restart of the machine leaves: a workspace with a session
/// of Claude Code that was running when everything was killed, and a Keeper started again
/// on what the first one kept, which lists the session as resumable. A client of the world
/// opens the repository and its launcher, as a Window would, and is gone before the
/// Keeper is killed; the stand-in on the world's search path follows the script `default`,
/// which holds at [`CutOff::gate`] and then ends. The world has a Keeper running when this
/// returns, and no Window is open on it.
pub fn a_session_cut_off(world: &mut World) -> CutOff {
    world.agents_on_path();
    let gate = world.home().with_file_name("cut-off-gate");
    world.agent_script(
        "default",
        &Script(vec![Step::Hold(gate.clone()), Step::Exit]),
    );
    world.start_keeper();
    let repo = repository_to_review(world, "cut-off");
    let mut client = world.client();
    let (_, workspace) = client
        .open_folder(repo.path())
        .expect("the Keeper opens the folder");
    client
        .open_launcher(&workspace, "claude", CLIENT_SIZE)
        .expect("the Keeper opens the launcher");
    let session = session_where(&mut client, &workspace, |session| {
        session.state != SessionState::NotRunning
    });
    // The id the CLI gave itself is written down when the stand-in starts.
    assert!(
        wait_for(DEADLINE, || !world.agent_log("default").session.is_empty()),
        "the stand-in wrote down no session id"
    );
    let cli_session = world.agent_log("default").session;
    drop(client);
    world.kill_everything();
    world.start_keeper();
    let mut client = world.client();
    session_where(&mut client, &workspace, |listed| {
        listed.id == session.id && listed.resumable
    });
    drop(client);
    CutOff {
        repo,
        workspace,
        session: session.id,
        cli_session,
        gate,
    }
}

/// The session of `workspace` that `wanted` accepts, once the Keeper lists one: the list is
/// asked for again until it does.
fn session_where(
    client: &mut yaar_system_tests::Client,
    workspace: &WorkspaceId,
    mut wanted: impl FnMut(&yaar_link::Session) -> bool,
) -> yaar_link::Session {
    let mut found = None;
    let came = wait_for(DEADLINE, || {
        found = client
            .agents()
            .into_iter()
            .filter(|agents| agents.workspace == *workspace)
            .flat_map(|agents| agents.sessions)
            .find(|session| wanted(session));
        found.is_some()
    });
    found
        .filter(|_| came)
        .unwrap_or_else(|| panic!("no such session came: {:?}", client.agents()))
}

/// Keeps the app running until `holds` says yes. Fails at the deadline, naming `what`.
pub fn wait_until(
    cx: &mut VisualTestContext,
    what: &str,
    mut holds: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain for {what}");
        std::thread::park_timeout(PAUSE);
    }
}

/// Where the element drawn under `selector` was laid out in the last frame, if one was.
pub fn drawn(
    selector: &str,
    cx: &mut VisualTestContext,
) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    // The test context takes a text that lives as long as the program.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

/// The key, as the table binds it on this system, of the control of the Window titled
/// `title`.
pub fn key_of(title: &str) -> String {
    yaar_window::controls()
        .into_iter()
        .find(|control| control.title == title)
        .unwrap_or_else(|| panic!("no control is titled {title:?}"))
        .key()
}

/// The control of the Window with this key, as the table binds it on this system.
pub fn control_of(key: &str) -> Option<Control> {
    yaar_window::controls()
        .into_iter()
        .find(|control| control.key() == key)
}

impl Composed {
    /// Waits for an element that is drawn under `selector`, and gives the point `inset`
    /// right of its left edge, in the middle of its height.
    pub fn point_in(
        &self,
        selector: &str,
        cx: &mut VisualTestContext,
    ) -> gpui_kit::Point<gpui_kit::Pixels> {
        let mut found = None;
        wait_until(cx, selector, |cx| {
            found = drawn(selector, cx);
            found.is_some()
        });
        let bounds = found.expect("the element is drawn");
        point(bounds.left() + px(2.), bounds.center().y)
    }

    /// Clicks on the bar of the review pane, which puts the focus in the pane whichever view
    /// it shows (Flow draws no first row) and pauses no follow.
    pub fn click_into_the_review_pane(&self, cx: &mut VisualTestContext) {
        let at = self.point_in("review-bar", cx);
        cx.simulate_click(at, Modifiers::none());
    }

    /// Clicks at the left end of an element, once it is drawn.
    pub fn click_into(&mut self, cx: &mut VisualTestContext, selector: &'static str) {
        let mut found = None;
        wait_until(cx, selector, |cx| {
            found = cx.debug_bounds(selector);
            found.is_some()
        });
        let found = found.expect("the element is drawn");
        cx.simulate_click(
            point(found.left() + px(2.), found.center().y),
            Modifiers::none(),
        );
    }

    /// Shows Changes in the review pane by a click on its name in the bar, which pauses no
    /// follow, and waits until it is on screen with its first row drawn.
    pub fn show_the_changes_view(&self, cx: &mut VisualTestContext) {
        let at = self.point_in("review-view-changes", cx);
        cx.simulate_click(at, Modifiers::none());
        wait_until(cx, "Changes on screen", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.view() == yaar_review_pane::View::Changes)
                && cx.debug_bounds("review-row-0").is_some()
        });
    }

    /// Whether some line of the screen of `terminal`, as its emulator holds it, holds `mark`.
    /// What a program prints is read from the screen, as a user reads it; a shell's prompt
    /// is no mark, so a test waits for a mark the program prints itself.
    pub fn shows_on_screen(
        &self,
        terminal: &TerminalId,
        mark: &str,
        cx: &mut VisualTestContext,
    ) -> bool {
        self.terminals
            .read_with(cx, |pane, _| pane.terminal(terminal))
            .is_some_and(|view| {
                view.read_with(cx, |view, _| view.lines())
                    .iter()
                    .any(|line| line.contains(mark))
            })
    }

    /// Whether the focus is inside an element that sets this key context.
    pub fn focus_is_in(&self, context: &str, cx: &mut VisualTestContext) -> bool {
        cx.update(|window, _| {
            window
                .context_stack()
                .iter()
                .any(|stack| stack.contains(context))
        })
    }

    /// Ends the Navigator the Window started, as the quit of the app does, and proves it
    /// gone: it ended within its deadline, and by the time the call returns the process of
    /// no server it had told is left (the call waited for them: nothing here does). Does
    /// nothing for a Window opened with none.
    pub fn end_the_navigator(&mut self, cx: &mut VisualTestContext) {
        let pids: Vec<u32> = self.models.navigation.read_with(cx, |navigation, _| {
            navigation
                .servers()
                .iter()
                .filter_map(|server| server.pid)
                .collect()
        });
        let ended = cx.update(|_, cx| yaar_window::stop_navigator(&self.models, cx));
        assert!(ended, "the Navigator did not end within its deadline");
        assert!(
            pids.iter().all(|pid| !is_alive(*pid)),
            "a language server outlived the end of the Navigator: {:?}",
            pids.iter()
                .filter(|pid| is_alive(**pid))
                .collect::<Vec<_>>()
        );
    }

    /// Gives the world back and lets go of the rest of this Window: its tap ends first,
    /// since a tap that outlived its world could not be woken to end once the world's
    /// folder was removed. The Window itself stays on the test platform, with its link
    /// lost once the world's Keeper ends. For a test that ends the Keeper and starts
    /// another in the same world, and opens a second Window against it; the Keeper must
    /// hold no program, since this waits until the programs it listed have ended.
    pub fn into_world(self) -> World {
        let Composed { world, tap, .. } = self;
        drop(tap);
        world
    }

    /// A number no earlier call gave, for a name of the test's own.
    pub fn next(&mut self) -> u32 {
        self.made += 1;
        self.made
    }

    /// A new repository of the world with one change to review, opened as a workspace by
    /// the key of Open Folder and the dialog, as a user does it. Returns its folder, once
    /// the workspace is on screen and the review pane draws its first row.
    pub fn open_a_folder(&mut self, cx: &mut VisualTestContext) -> PathBuf {
        let name = format!("opened-{}", self.next());
        let repository = repository_to_review(&self.world, &name);
        let folder = repository.path().to_owned();
        self.repositories.push(repository);
        self.open_this_folder(&folder, cx);
        folder
    }

    /// Opens `folder` as a workspace by the key of Open Folder and the dialog, as a user
    /// does it, and waits until the workspace is on screen and the review pane draws its
    /// first row.
    pub fn open_this_folder(&mut self, folder: &std::path::Path, cx: &mut VisualTestContext) {
        let folder = folder.to_owned();
        cx.simulate_keystrokes(&key_of("Open Folder…"));
        assert!(
            cx.did_prompt_for_paths(),
            "the key of Open Folder asks for a folder"
        );
        let chosen = folder.clone();
        cx.simulate_path_prompt_response(move |_| Some(vec![chosen.clone()]));
        wait_until(cx, "the opened folder on screen", |cx| {
            self.models.workspaces.read_with(cx, |list, _| {
                list.selected()
                    .and_then(|selected| list.workspace(selected))
                    .is_some_and(|workspace| workspace.path == folder)
            })
        });
        wait_until(cx, "the first row of the review pane", |cx| {
            cx.debug_bounds("review-row-0").is_some()
        });
    }

    /// Starts the stand-in in a launcher terminal of the workspace on screen, as the offer
    /// made on attaching does, and has the Window show what the keys of the reviews act on:
    /// that terminal focused, and the review of its session in the review pane. The agent
    /// is kept in [`Composed::agent`].
    pub fn start_an_agent(&mut self, cx: &mut VisualTestContext) {
        let workspace = self
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected().cloned())
            .expect("a workspace is on screen before an agent is started");
        // What the review pane asks for when its offer to start an agent is taken: the
        // launcher of the CLI for the review shown, the one of no agent for now. The pane
        // of the terminals opens it and gives it the focus.
        let mut shown = None;
        wait_until(cx, "the review pane to hold a review", |cx| {
            shown = self
                .review
                .read_with(cx, |pane, _| pane.review().map(|review| review.id.clone()));
            shown.is_some()
        });
        let review_of_no_agent = shown.expect("the pane holds a review");
        self.models.terminals.update(cx, |list, cx| {
            list.want(
                Wanted::AgentFor {
                    workspace,
                    agent: "claude".to_owned(),
                    review: review_of_no_agent,
                },
                cx,
            )
        });
        let mut found = None;
        wait_until(
            cx,
            "the session in the terminal that took the focus, with its review",
            |cx| {
                found = self
                    .terminals
                    .read_with(cx, |pane, _| pane.focused())
                    .and_then(|terminal| {
                        self.models
                            .agents
                            .read_with(cx, |agents, _| agents.in_terminal(&terminal).cloned())
                    });
                found
                    .as_ref()
                    .is_some_and(|session| session.review.is_some())
            },
        );
        let session = found.expect("the agents list the session");
        let review = session.review.clone().expect("it has a review");
        let terminal = session.terminal.clone().expect("it runs in a terminal");
        wait_until(cx, "the focus in the launcher's terminal", |cx| {
            self.focus_is_in("Program", cx)
        });

        self.show_the_review_of(&session.id, cx);
        self.agent = Some(RunningAgent {
            terminal,
            session: session.id,
            review,
            thread: None,
        });
    }

    /// Brings the review of `session` into the review pane by Next Review, from the pane:
    /// the pane need not show it at first, and a key that went through the reviews has left
    /// another one up. At most once for each review there is.
    pub fn show_the_review_of(&mut self, session: &SessionId, cx: &mut VisualTestContext) {
        wait_until(cx, "the session among those the review pane names", |cx| {
            self.review.read_with(cx, |pane, _| {
                pane.review().is_some() && pane.sessions().iter().any(|s| s.id == *session)
            })
        });
        self.click_into_the_review_pane(cx);
        let shows = |window: &Self, cx: &mut VisualTestContext| {
            window
                .review
                .read_with(cx, |pane, _| pane.review_session().cloned())
                == Some(session.clone())
        };
        let reviews = self
            .review
            .read_with(cx, |pane, _| pane.sessions().len() + 1);
        for _ in 0..reviews {
            if shows(self, cx) {
                break;
            }
            cx.simulate_keystrokes(&key_of("Next Review"));
            cx.run_until_parked();
        }
        wait_until(
            cx,
            "the review of the agent's session in the review pane",
            |cx| shows(self, cx),
        );
    }
}
