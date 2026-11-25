#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyView, Entity, KeystrokeEvent, Modifiers, Subscription, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_frame::{BuildPane, Frame, LinkStatus, Models, Opened, PROGRAM_CONTEXT, Pane, Wanted};
use yaar_link::{
    ReviewId, SessionId, SessionState, TerminalId, TerminalSize, ThreadId, WorkspaceId,
};
use yaar_navigator::{NavigatorConfig, STOP_AFTER, SystemClock};
use yaar_review_pane::ReviewPane;
use yaar_system_tests::standin::{Script, Step};
use yaar_system_tests::{Repo, Tap, World, descendants_of, is_alive, wait_until as wait_for};
use yaar_terminal_pane::TerminalPane;

pub use yaar_system_tests::DEADLINE;

const PAUSE: Duration = Duration::from_millis(1);

pub struct Composed {
    // Drop order: the programs are listed while the Keeper runs, the tap goes before its world,
    // and once the Keeper has ended the test waits until the programs are gone too.
    _listing: ListingPrograms,
    pub tap: Tap,
    pub world: World,
    pub frame: Entity<Frame>,
    pub models: Models,
    pub review: Entity<ReviewPane>,
    pub terminals: Entity<TerminalPane>,
    /// The actions the keystrokes ran since the list was last taken; a key the focused element took
    /// runs none.
    pub ran: Rc<RefCell<Vec<&'static str>>>,
    _watching_the_keys: Subscription,
    pub repositories: Vec<Repo>,
    pub agent: Option<RunningAgent>,
    pub cut_off: Option<CutOff>,
    made: u32,
    _waiting: WaitingForPrograms,
}

pub struct RunningAgent {
    pub terminal: TerminalId,
    pub session: SessionId,
    pub review: ReviewId,
    pub thread: Option<ThreadId>,
}

/// Listed while the Keeper runs: once it is gone its children belong to the system.
struct ListingPrograms {
    keeper: u32,
    into: Rc<RefCell<Vec<u32>>>,
}

impl Drop for ListingPrograms {
    fn drop(&mut self) {
        *self.into.borrow_mut() = descendants_of(self.keeper);
    }
}

struct WaitingForPrograms {
    started: Rc<RefCell<Vec<u32>>>,
}

impl Drop for WaitingForPrograms {
    fn drop(&mut self) {
        let started = self.started.borrow().clone();
        let gone = wait_for(DEADLINE, || started.iter().all(|pid| !is_alive(*pid)));
        // A second panic in the unwinding of a failed test would end the process before it says
        // why.
        if !gone && !std::thread::panicking() {
            let left: Vec<u32> = started.into_iter().filter(|pid| is_alive(*pid)).collect();
            panic!("the programs of the Keeper did not end with it: {left:?}");
        }
    }
}

pub fn open_composed(
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    world.start_keeper();
    open_composed_in(world, cx, extra)
}

pub fn open_composed_in(
    world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
) -> (Composed, VisualTestContext) {
    open_composed_with(world, cx, extra, true, None)
}

pub fn open_another_composed_in(
    world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
) -> (Composed, VisualTestContext) {
    open_composed_with(world, cx, extra, false, None)
}

pub fn open_composed_with_a_navigator(
    world: World,
    cx: &mut TestAppContext,
    extra: Vec<BuildPane>,
    navigator: NavigatorConfig,
) -> (Composed, VisualTestContext) {
    open_composed_with(world, cx, extra, true, Some(navigator))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    OnTheMachine,
    /// `tools/standin-lsp` in the repository; a server inside a repository runs only after the user
    /// said yes.
    InTheProject,
}

fn navigator_data(world: &World) -> PathBuf {
    world.home().with_file_name("navigator-data")
}

/// The Keeper writes an export only in the cache folder of its own environment.
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

/// One registry row, `standin`, for the invented `.sil` language and `extra_extensions`, started
/// when the worktree holds `marker`.
pub fn standin_row(found: Found, marker: &str, extra_extensions: &[&str]) -> String {
    standin_row_started_with(found, marker, extra_extensions, &[])
}

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

pub fn navigator_for(world: &World, found: Found, row: &str) -> NavigatorConfig {
    let data = navigator_data(world);
    let folder = data.join("machine");
    std::fs::create_dir_all(&folder).expect("make the folder of the machine's programs");
    let mut search_path = Vec::new();
    if found == Found::OnTheMachine {
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
        login_shell: None,
        platform: None,
        stop_after: STOP_AFTER,
        clock: std::sync::Arc::new(SystemClock),
        auto_download: false,
    }
}

/// Kept out of what git sees (the exclude file): the worktree has the file, the review has no row
/// for it.
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

    // A real socket and a real process: a thread and a program the test executor cannot see.
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
            Some(navigator) => yaar_window::open_panes(tap.socket(), builders, navigator, cx),
            None => yaar_frame::open(tap.socket(), yaar_window::VERSION, builders, cx),
        })
        .expect("open the composed Window on the test platform");
    let mut cx = VisualTestContext::from_window(window, cx);
    // The window a user types in is the active one; GPUI tells no focus change to a window that is
    // not.
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

/// In a world git reads no global configuration: macOS guesses an identity, Linux refuses the
/// commit.
pub fn give_an_identity(repository: &Repo) {
    repository.git(&["config", "user.name", "A Test"]);
    repository.git(&["config", "user.email", "test@example.invalid"]);
}

pub fn repository_to_review(world: &World, name: &str) -> Repo {
    let repository = world.repository(name);
    give_an_identity(&repository);
    repository.git(&["checkout", "--quiet", "-b", "work"]);
    repository.write("notes.txt", "one\ntwo\nthree\n");
    repository.commit("Add the notes");
    repository.write("notes.txt", "one\n2\nthree\n");
    repository
}

pub struct CutOff {
    pub repo: Repo,
    pub workspace: WorkspaceId,
    pub session: SessionId,
    pub cli_session: String,
    pub gate: PathBuf,
}

const CLIENT_SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// Brings `world` to the state a restart leaves: a resumable Claude Code session, no Window open.
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

pub fn wait_until(
    cx: &mut VisualTestContext,
    what: &str,
    holds: impl FnMut(&mut VisualTestContext) -> bool,
) {
    wait_until_telling(cx, what, holds, |_| None);
}

/// On failure also says what `held` reads of the app, to tell a wait that failed on a loaded
/// machine from a fault.
pub fn wait_until_telling(
    cx: &mut VisualTestContext,
    what: &str,
    mut holds: impl FnMut(&mut VisualTestContext) -> bool,
    held: impl FnOnce(&mut VisualTestContext) -> Option<String>,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        if Instant::now() >= deadline {
            match held(cx) {
                Some(held) => panic!("waited in vain for {what}; at the deadline {held}"),
                None => panic!("waited in vain for {what}"),
            }
        }
        std::thread::park_timeout(PAUSE);
    }
}

pub fn drawn(
    selector: &str,
    cx: &mut VisualTestContext,
) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    // The test context takes a text that lives as long as the program.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}

pub fn key_of(title: &str) -> String {
    yaar_window::controls()
        .into_iter()
        .find(|control| control.title == title)
        .unwrap_or_else(|| panic!("no control is titled {title:?}"))
        .key()
}

const ROWS_TOLD: usize = 12;

impl Composed {
    pub fn what_the_review_pane_holds(&self, cx: &mut VisualTestContext) -> String {
        let (view, notice, references, rows) = self.review.read_with(cx, |pane, _| {
            (
                pane.view(),
                pane.notice(),
                pane.references()
                    .map(|list| (list.waiting, list.locations.len(), list.guessed)),
                pane.rows().to_vec(),
            )
        });
        let named: Vec<usize> = (0..rows.len())
            .filter(|row| {
                ["new", "old"]
                    .into_iter()
                    .any(|side| drawn(&format!("review-text-{row}-{side}:three"), cx).is_some())
            })
            .collect();
        format!(
            "the review pane showed {view:?} with the notice {notice:?} and the list of \
             references {references:?} (waiting, found, guessed); it held {} rows, of which the \
             first {ROWS_TOLD} were {:?}; a name of the notes was drawn on the rows {named:?}; \
             the Window's last requests, newest first, were {:?}",
            rows.len(),
            rows.iter().take(ROWS_TOLD).collect::<Vec<_>>(),
            self.tap.requests().iter().rev().take(6).collect::<Vec<_>>()
        )
    }

    pub fn what_the_frame_and_the_terminals_show(&self, cx: &mut VisualTestContext) -> String {
        let (on_screen, listed) = self.models.workspaces.read_with(cx, |list, _| {
            let listed: Vec<String> = list
                .repositories()
                .iter()
                .flat_map(|repository| {
                    repository.workspaces.iter().map(|workspace| {
                        format!("{} {:?}", workspace.path.display(), workspace.head)
                    })
                })
                .collect();
            (list.selected().cloned(), listed)
        });
        let notice = self.frame.read_with(cx, |frame, cx| frame.notice(cx));
        let (terminals_notice, focused) = self
            .terminals
            .read_with(cx, |pane, _| (pane.notice(), pane.focused()));
        let lines = focused
            .and_then(|id| self.terminals.read_with(cx, |pane, _| pane.terminal(&id)))
            .map(|view| view.read_with(cx, |view, _| view.lines()))
            .map(|lines| {
                lines
                    .into_iter()
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
            });
        format!(
            "the workspaces listed were {listed:?} with {on_screen:?} on screen; the Frame's \
             notice was {notice:?}; the terminal pane's notice was {terminals_notice:?} and the \
             lines of its focused terminal were {lines:?}; the Window's last requests, newest first, were {:?}",
            self.tap.requests().iter().rev().take(6).collect::<Vec<_>>()
        )
    }

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

    /// Focuses the pane whichever view it shows (Flow draws no first row) and pauses no follow.
    pub fn click_into_the_review_pane(&self, cx: &mut VisualTestContext) {
        let at = self.point_in("review-bar", cx);
        cx.simulate_click(at, Modifiers::none());
    }

    /// Off macOS the app's keys that are not kept belong to the program while a terminal has the
    /// focus.
    pub fn leave_the_terminal_off_macos(&self, cx: &mut VisualTestContext) {
        if cfg!(target_os = "macos") || !self.focus_is_in(PROGRAM_CONTEXT, cx) {
            return;
        }
        self.click_into_the_review_pane(cx);
        wait_until(cx, "the focus out of the terminal", |cx| {
            !self.focus_is_in(PROGRAM_CONTEXT, cx)
        });
    }

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

    pub fn show_the_changes_view(&self, cx: &mut VisualTestContext) {
        let at = self.point_in("review-view-changes", cx);
        cx.simulate_click(at, Modifiers::none());
        wait_until(cx, "Changes on screen", |cx| {
            let (in_changes, empty) = self.review.read_with(cx, |pane, _| {
                (
                    pane.view() == yaar_review_pane::View::Changes,
                    pane.rows().is_empty(),
                )
            });
            in_changes && (empty || cx.debug_bounds("review-row-0").is_some())
        });
    }

    /// A shell's prompt is no mark: wait for one the program prints itself.
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

    pub fn focus_is_in(&self, context: &str, cx: &mut VisualTestContext) -> bool {
        cx.update(|window, _| {
            window
                .context_stack()
                .iter()
                .any(|stack| stack.contains(context))
        })
    }

    /// Quits the app, which ends the Navigator, and asserts that none of its servers outlived the
    /// quit. The app has quit afterwards, so a test calls it last.
    pub fn end_the_navigator(&mut self, cx: &mut VisualTestContext) {
        let pids: Vec<u32> = self.models.navigation.read_with(cx, |navigation, _| {
            navigation
                .servers()
                .iter()
                .filter_map(|server| server.pid)
                .collect()
        });
        cx.quit();
        assert!(
            pids.iter().all(|pid| !is_alive(*pid)),
            "a language server outlived the quit of the app: {:?}",
            pids.iter()
                .filter(|pid| is_alive(**pid))
                .collect::<Vec<_>>()
        );
    }

    /// The tap is dropped before the world: a tap that outlived its world could not be woken. The
    /// Keeper must hold no program, since dropping the rest waits until the listed ones have ended.
    pub fn into_world(self) -> World {
        let Composed { world, tap, .. } = self;
        drop(tap);
        world
    }

    pub fn next(&mut self) -> u32 {
        self.made += 1;
        self.made
    }

    pub fn open_a_folder(&mut self, cx: &mut VisualTestContext) -> PathBuf {
        let name = format!("opened-{}", self.next());
        let repository = repository_to_review(&self.world, &name);
        let folder = repository.path().to_owned();
        self.repositories.push(repository);
        self.open_this_folder(&folder, cx);
        folder
    }

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
            let (loaded, empty) = self.review.read_with(cx, |pane, _| {
                (pane.comparison().is_some(), pane.rows().is_empty())
            });
            loaded && (empty || cx.debug_bounds("review-row-0").is_some())
        });
    }

    pub fn start_an_agent(&mut self, cx: &mut VisualTestContext) {
        let workspace = self
            .models
            .workspaces
            .read_with(cx, |list, _| list.selected().cloned())
            .expect("a workspace is on screen before an agent is started");
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
