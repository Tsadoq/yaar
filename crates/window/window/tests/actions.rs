//! Every control of the Window's table is pressed as a user presses it; the requests are read on a
//! tap before the Keeper.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    Action, AppContext, Context, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext,
    Window, WindowOptions, div,
};
use yaar_frame::settings_screen::SettingRow;
use yaar_frame::{
    Asks, Beside, CloseWindow, Control, PROGRAM_CONTEXT, Place, SETTINGS_CONTEXT, Sends,
};
use yaar_link::{
    Attachment, CommentId, ConsentAnswer, Fact, FlowForm, Head, Origin, Program, Refusal, Request,
    RequestKind, ReviewId, Setting, TerminalPlace, ThreadId, ThreadState, TurnEndSelects,
    Workspace, WorkspaceId,
};
use yaar_review_pane::{PaneRow, View};
use yaar_system_tests::standin::{Did, Script};
use yaar_system_tests::{DEADLINE, Repo, World, descendants_of, is_alive};

mod common;

use common::{Composed, Found, RunningAgent, key_of, wait_until, wait_until_telling};

thread_local! {
    static ECHOES: Rc<RefCell<Vec<Setting>>> = Rc::default();
}

type Wanted = Box<dyn Fn(&Request) -> bool>;

const DRAFT: &str = "walked";

const COMMIT_MESSAGE: &str = "Walked commit";

#[derive(Debug)]
enum Answer {
    Nothing,
    Folder(PathBuf),
    Text(String),
}

fn open_composed(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    world.start_keeper();
    open_with_a_navigator(world, cx)
}

fn open_with_a_navigator(world: World, cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let navigator = common::navigator_for(
        &world,
        Found::InTheProject,
        &common::standin_row(Found::InTheProject, "notes.txt", &["txt"]),
    );
    let (window, mut cx) = check_the_panes(common::open_composed_with_a_navigator(
        world,
        cx,
        Vec::new(),
        navigator,
    ));
    hear_the_settings(&window, &mut cx);
    (window, cx)
}

/// The Keeper echoes a setting to every Window, the setter included: after two quick sets the
/// first echo shows the first value again until the second comes. A step that reads a setting
/// waits for every echo first.
fn hear_the_settings(window: &Composed, cx: &mut VisualTestContext) {
    let echoes = ECHOES.with(Rc::clone);
    echoes.borrow_mut().clear();
    cx.update(|_, app| {
        app.subscribe(&window.models.link, move |_, fact: &Fact, _| {
            if let Fact::SettingChanged(setting) = fact {
                echoes.borrow_mut().push(setting.clone());
            }
        })
        .detach();
    });
}

fn open_composed_with_an_agent(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    world.agents_on_path();
    let script = Script::parse(&format!("hold {}\nexit", gate_of(&world).display()));
    world.agent_script("default", &script);
    world.start_keeper();
    open_with_a_navigator(world, cx)
}

fn open_composed_for_the_walk(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    // pi's own folder is there before the first Keeper starts: the question exists only then.
    world.install_pi();
    let cut_off = common::a_session_cut_off(&mut world);
    let script = Script::parse(&format!("hold {}\nexit", gate_of(&world).display()));
    world.agent_script("default", &script);
    let (mut window, cx) = open_with_a_navigator(world, cx);
    window.cut_off = Some(cut_off);
    (window, cx)
}

fn gate_of(world: &World) -> PathBuf {
    world.home().with_file_name("gate")
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn check_the_panes(opened: (Composed, VisualTestContext)) -> (Composed, VisualTestContext) {
    let (window, cx) = opened;
    let listed: Vec<&str> = yaar_window::panes().iter().map(|pane| pane.name).collect();
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.panes()),
        listed
    );
    (window, cx)
}

fn place_of(beside: Beside) -> TerminalPlace {
    match beside {
        Beside::Right => TerminalPlace::Right,
        Beside::Below => TerminalPlace::Below,
        Beside::Left => TerminalPlace::Left,
    }
}

struct Spare;

impl Render for Spare {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn open_a_spare_window(cx: &mut VisualTestContext) -> gpui_kit::AnyWindowHandle {
    cx.update(|_, app| {
        let spare: gpui_kit::AnyWindowHandle = app
            .open_window(WindowOptions::default(), |_, app| app.new(|_| Spare))
            .expect("open the spare window")
            .into();
        spare
            .update(app, |_, window, _| window.activate_window())
            .expect("activate the spare window");
        assert_eq!(
            app.active_window(),
            Some(spare),
            "the spare window is active"
        );
        spare
    })
}

fn controls_that_stop() -> Vec<Control> {
    yaar_window::controls()
        .into_iter()
        .filter(|control| control.sends == Some(Sends::StopEverything))
        .collect()
}

fn controls_that_send(kind: RequestKind) -> Vec<Control> {
    yaar_window::controls()
        .into_iter()
        .filter(|control| control.sends == Some(Sends::Request(kind)))
        .collect()
}

#[derive(PartialEq, Eq)]
struct Bytes(Vec<u8>);

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} bytes: {:?}",
            self.0.len(),
            String::from_utf8_lossy(&self.0)
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Reads {
    git: Vec<Bytes>,
    objects: Vec<(PathBuf, u64)>,
}

fn read_git(home: &Path, folder: &Path, args: &[&str]) -> Vec<u8> {
    let mut git = Command::new("git");
    git.env_clear();
    if let Some(search) = std::env::var_os("PATH") {
        git.env("PATH", search);
    }
    let output = git
        .current_dir(folder)
        .args(args)
        .env("HOME", home)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z")
        .env("GIT_COMMITTER_NAME", "A Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn files_below(folder: &Path, below: &Path, found: &mut Vec<(PathBuf, u64)>) {
    for entry in fs::read_dir(folder).expect("read the folder") {
        let path = entry.expect("an entry").path();
        let meta = fs::symlink_metadata(&path).expect("the kind");
        if meta.is_dir() {
            files_below(&path, below, found);
        } else {
            found.push((
                path.strip_prefix(below).expect("below").to_owned(),
                meta.len(),
            ));
        }
    }
}

fn reads(home: &Path, repo: &Repo) -> Reads {
    let git_folder = repo.path().join(".git");
    let mut objects = Vec::new();
    files_below(
        &git_folder.join("objects"),
        &git_folder.join("objects"),
        &mut objects,
    );
    objects.sort();
    let git = [
        &[
            "status",
            "--porcelain=v2",
            "--ignored",
            "--untracked-files=all",
        ][..],
        &["for-each-ref"],
        &["worktree", "list", "--porcelain"],
        &["stash", "list"],
        &["fsck", "--unreachable", "--no-reflogs"],
    ]
    .iter()
    .map(|args| read_git(home, repo.path(), args))
    .chain([
        fs::read(git_folder.join("index")).expect("the index"),
        fs::read(git_folder.join("config")).expect("the configuration"),
    ])
    .map(Bytes)
    .collect();
    Reads { git, objects }
}

fn plant(home: &Path, repo: &Repo) -> Vec<String> {
    let file = home.join("planted.txt");
    fs::write(&file, "a blob nothing refers to\n").expect("write the blob's file");
    let blob = repo.git(&["hash-object", "-w", file.to_str().expect("a path of text")]);
    repo.write("README.md", "edited\n");
    let dangling = repo.git(&["stash", "create"]);
    repo.git(&["checkout", "--quiet", "--", "README.md"]);
    assert!(!dangling.is_empty(), "the edit left no commit to plant");
    vec![blob, dangling]
}

fn still_there(repo: &Repo, planted: &[String]) {
    for id in planted {
        repo.git(&["cat-file", "-e", id]);
    }
}

impl Composed {
    fn on_screen(&self, cx: &mut VisualTestContext) -> Option<Workspace> {
        self.models.workspaces.read_with(cx, |list, _| {
            list.selected()
                .and_then(|selected| list.workspace(selected))
                .cloned()
        })
    }

    fn new_repository(&mut self) -> PathBuf {
        let name = format!("walked-{}", self.next());
        let repository = common::repository_to_review(&self.world, &name);
        common::put_the_standin_in(&repository);
        let folder = repository.path().to_owned();
        self.repositories.push(repository);
        folder
    }

    fn repository_on_screen(&self, cx: &mut VisualTestContext) -> &Repo {
        let on_screen = self
            .on_screen(cx)
            .expect("a workspace is on screen before the walk looks at its repository");
        self.repositories
            .iter()
            .find(|repository| repository.path() == on_screen.path)
            .expect("the workspace on screen is one the walk opened")
    }

    fn new_branch_on_screen(&mut self, cx: &mut VisualTestContext) -> (Workspace, String) {
        let on_screen = self
            .on_screen(cx)
            .expect("a workspace is on screen before the walk sets a base");
        let branch = format!("base-{}", self.next());
        self.repository_on_screen(cx).git(&["branch", &branch]);
        (on_screen, branch)
    }

    /// A Keeper killed mid-answer leaves its copy of git's index in the system's temp folder, so
    /// each step waits for what its request asked. Review requests are answered in order, so
    /// waiting for the last covers those before it.
    fn settle(
        &mut self,
        kind: RequestKind,
        passed: &Request,
        from: usize,
        cx: &mut VisualTestContext,
    ) {
        match kind {
            RequestKind::CopyReview => wait_until(cx, "the copied review on the clipboard", |cx| {
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .is_some_and(|text| text.contains(DRAFT))
            }),
            RequestKind::MarkReviewed => wait_until(cx, "the review marked reviewed", |cx| {
                self.review.read_with(cx, |pane, _| {
                    pane.review()
                        .is_some_and(|review| review.reviewed.is_some())
                })
            }),
            RequestKind::SetViewed => wait_until(cx, "the file shown viewed", |cx| {
                self.review.read_with(cx, |pane, _| {
                    pane.reviews()
                        .is_some_and(|reviews| reviews.viewed.contains(&PathBuf::from("notes.txt")))
                })
            }),
            RequestKind::Commit => {
                let repository = self.repository_on_screen(cx);
                wait_until(cx, "the commit in the repository", |_| {
                    repository.git(&["log", "-1", "--format=%s"]) == COMMIT_MESSAGE
                });
            }
            // The pane takes the focus for a terminal when the Keeper's answer is taken, after the
            // request passed the tap.
            RequestKind::OpenTerminal => {
                wait_until(cx, "the new terminal to take the focus", |cx| {
                    self.focus_is_in("Program", cx)
                });
                wait_until(cx, "the new terminal on screen", |cx| {
                    cx.debug_bounds("terminal-slot-0").is_some()
                });
            }
            RequestKind::SetSetting => {
                let Request::SetSetting { setting } = passed else {
                    panic!("the step of SetSetting waited for {passed:?}")
                };
                self.wait_for_the_echoes(setting, cx);
                match passed {
                    Request::SetSetting {
                        setting: Setting::FlowForm { .. },
                    } => wait_until(cx, "the live diff as the form of Flow", |cx| {
                        self.review
                            .read_with(cx, |pane, _| pane.flow_form() == FlowForm::LiveDiff)
                    }),
                    Request::SetSetting {
                        setting:
                            Setting::ServersInRepository {
                                repository,
                                allowed,
                            },
                    } => {
                        wait_until(
                            cx,
                            "the answer to the question in the Window's model",
                            |cx| {
                                self.models.navigation.read_with(cx, |navigation, cx| {
                                    navigation.consent(repository, cx) == Some(*allowed)
                                })
                            },
                        );
                        wait_until(cx, "the question to leave the line of the pane", |cx| {
                            cx.debug_bounds("review-servers-yes").is_none()
                        });
                        if *allowed {
                            wait_until(cx, "the server of the repository ready", |cx| {
                                cx.debug_bounds("review-servers-state:ready").is_some()
                            });
                        }
                    }
                    Request::SetSetting {
                        setting: Setting::TerminalPlace(place),
                    } => wait_until(cx, "the panes laid out as the setting says", |cx| {
                        self.frame
                            .read_with(cx, |frame, _| place_of(frame.beside()) == *place)
                    }),
                    Request::SetSetting {
                        setting: Setting::TurnEnd(selects),
                    } => wait_until(
                        cx,
                        "the review pane to follow what a turn end selects",
                        |cx| {
                            self.review.read_with(cx, |pane, _| {
                                pane.keeps_selection()
                                    == (*selects == TurnEndSelects::KeepSelection)
                            })
                        },
                    ),
                    other => panic!("the step of SetSetting waited for {other:?}"),
                }
            }
            RequestKind::OpenFolder | RequestKind::SetBase | RequestKind::DeleteDraft => {}
            RequestKind::ListWorkspaces
            | RequestKind::GetChanges
            | RequestKind::ReadFile
            | RequestKind::ResizeTerminal
            | RequestKind::GetReviews
            | RequestKind::SaveDraft
            | RequestKind::ListAgents
            | RequestKind::GetHistory
            | RequestKind::GetUncommitted
            | RequestKind::CompareVersions
            | RequestKind::ReadVersions
            | RequestKind::GetTurn
            | RequestKind::ShowWorkspace
            | RequestKind::ListTerminals
            | RequestKind::AttachTerminal
            | RequestKind::GetSettings
            | RequestKind::GetAttachment
            | RequestKind::ListClis
            | RequestKind::CompareForReview
            | RequestKind::ListConsents
            | RequestKind::ExportVersion
            | RequestKind::GetStateSizes => {
                panic!("the Window starts {kind:?} by itself: no user does")
            }
            RequestKind::Report => panic!("a piece of an agent CLI starts {kind:?}: no user does"),
            RequestKind::CreateWorktree => {
                let Request::CreateWorktree { branch, .. } = passed else {
                    panic!("the step of CreateWorktree waited for {passed:?}")
                };
                let mut made = None;
                wait_until_telling(
                    cx,
                    "the new worktree on screen",
                    |cx| {
                        made = self.on_screen(cx).filter(|shown| {
                            shown
                                .path
                                .file_name()
                                .is_some_and(|name| name == branch.as_str())
                        });
                        made.is_some()
                    },
                    |cx| Some(self.what_the_frame_and_the_terminals_show(cx)),
                );
                let made = made.expect("the worktree is on screen").id;
                wait_until(
                    cx,
                    "the agent's terminal of the new worktree to take the focus",
                    |cx| self.focus_is_in("Program", cx),
                );
                wait_until(cx, "the session of the agent in the new worktree", |cx| {
                    self.models.agents.read_with(cx, |agents, _| {
                        agents
                            .of(&made)
                            .is_some_and(|listed| !listed.sessions.is_empty())
                    })
                });
            }
            RequestKind::CleanUpStates => wait_until(cx, "the clean-up to end", |cx| {
                self.models.states.read_with(cx, |states, _| {
                    !states.cleaning() && states.sizes().is_some()
                })
            }),
            RequestKind::CloseTerminal => {
                let Request::CloseTerminal { terminal } = passed else {
                    panic!("the step of CloseTerminal waited for {passed:?}")
                };
                wait_until(cx, "the closed terminal to leave the pane", |cx| {
                    self.terminals
                        .read_with(cx, |pane, _| !pane.terminals().contains(terminal))
                });
            }
            RequestKind::AnswerConsent => {
                let Request::AnswerConsent { answer, .. } = passed else {
                    panic!("the step of AnswerConsent waited for {passed:?}")
                };
                let settings = self.world.pi_settings();
                let lists_it = move || {
                    std::fs::read_to_string(&settings).is_ok_and(|text| text.contains("yaar.ts"))
                };
                let wanted = *answer == ConsentAnswer::Yes;
                wait_until(
                    cx,
                    "pi's settings and the Window to show the answer",
                    |cx| {
                        lists_it() == wanted
                            && self
                                .models
                                .consents
                                .read_with(cx, |consents, _| consents.asked().is_empty())
                    },
                );
                if !wanted {
                    let requests = self.tap.requests();
                    let at = requests
                        .iter()
                        .rposition(|request| request == passed)
                        .expect("the request that passed is on the tap");
                    assert!(
                        requests[from..at]
                            .iter()
                            .any(|request| request.kind() == RequestKind::ListConsents),
                        "the answer with no question shown was sent without asking who asks"
                    );
                }
            }
            RequestKind::AttachReview => {
                let attached = match passed {
                    Request::AttachReview {
                        thread: Some(thread),
                        ..
                    } => Attachment::Thread(thread.clone()),
                    _ => Attachment::Review,
                };
                self.wait_for_attachment(Some(attached), cx);
            }
            RequestKind::DetachReview => self.wait_for_attachment(None, cx),
            RequestKind::ResolveThread => self.wait_for_thread(ThreadState::Resolved, true, cx),
            RequestKind::ReopenThread => self.wait_for_thread(ThreadState::Resolved, false, cx),
            RequestKind::ResolveAddressed => {}
        }
    }

    fn wait_for_attachment(&mut self, attached: Option<Attachment>, cx: &mut VisualTestContext) {
        let terminal = self.agent().terminal.clone();
        wait_until(cx, "what is attached to show as the step did it", |cx| {
            self.review.read_with(cx, |pane, _| {
                pane.review().map(|review| review.attached.clone())
            }) == Some(attached.clone())
                && self
                    .terminals
                    .read_with(cx, |pane, cx| pane.attachment(&terminal, cx))
                    == attached
        });
    }

    fn wait_for_thread(&mut self, state: ThreadState, is: bool, cx: &mut VisualTestContext) {
        let thread = self
            .agent()
            .thread
            .clone()
            .expect("the walk wrote a thread");
        wait_until(cx, "the thread in the state the step put it in", |cx| {
            self.review.read_with(cx, |pane, _| {
                pane.thread_states()
                    .iter()
                    .any(|(id, now)| *id == thread && (*now == state) == is)
            })
        });
    }

    fn agent(&self) -> &RunningAgent {
        self.agent.as_ref().expect("the walk started an agent")
    }

    fn review_shown(&self, cx: &mut VisualTestContext) -> ReviewId {
        let mut found = None;
        wait_until(cx, "the review pane to hold a review", |cx| {
            found = self
                .review
                .read_with(cx, |pane, _| pane.review().map(|review| review.id.clone()));
            found.is_some()
        });
        found.expect("the pane holds a review")
    }

    /// Writes a draft on the reviewer's line as a user does. The pane is put in Changes first,
    /// whichever view an earlier step left, since Flow draws no row to click.
    fn write_a_draft(&mut self, cx: &mut VisualTestContext) -> (ReviewId, CommentId) {
        self.write_a_draft_lower(0, cx)
    }

    fn write_a_draft_lower(
        &mut self,
        down: usize,
        cx: &mut VisualTestContext,
    ) -> (ReviewId, CommentId) {
        self.show_changes(cx);
        self.review_shown(cx);
        wait_until(cx, "the reviewer on a line", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.position().is_some())
        });
        self.click_into(cx, "review-row-0");
        for _ in 0..down {
            cx.simulate_keystrokes("j");
        }
        cx.simulate_keystrokes("c");
        wait_until(cx, "the comment box", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.comment_box().is_some())
        });
        cx.simulate_input(DRAFT);
        cx.simulate_keystrokes("escape");
        let mut found = None;
        wait_until(cx, "the draft in the review pane's model", |cx| {
            found = self.review.read_with(cx, |pane, _| {
                let review = pane.review()?;
                review
                    .threads
                    .iter()
                    .flat_map(|thread| &thread.comments)
                    .find(|comment| comment.text == DRAFT && comment.round.is_none())
                    .map(|comment| (review.id.clone(), comment.id.clone()))
            });
            found.is_some()
        });
        found.expect("the pane holds the draft")
    }

    fn show_changes(&mut self, cx: &mut VisualTestContext) {
        self.click_into(cx, "review-view-changes");
        wait_until(cx, "Changes on screen", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.view() == View::Changes)
                && cx.debug_bounds("review-row-0").is_some()
        });
    }

    fn show_flow(&mut self, cx: &mut VisualTestContext) {
        self.click_into(cx, "review-view-flow");
        wait_until(cx, "Flow on screen", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.view() == View::Flow)
        });
    }

    /// Off macOS a key that works anywhere does nothing in a terminal, so the focus goes to the
    /// review pane first.
    fn focus_where_the_key_works(&mut self, control: &Control, cx: &mut VisualTestContext) {
        if control.place == Place::In(SETTINGS_CONTEXT) {
            return self.open_the_settings(cx);
        }
        self.close_the_settings(cx);
        match control.place {
            Place::Kept => {}
            Place::Anywhere => {
                if self.focus_is_in(PROGRAM_CONTEXT, cx) {
                    self.click_into(cx, "review-bar");
                    wait_until(cx, "the focus out of the terminal", |cx| {
                        !self.focus_is_in(PROGRAM_CONTEXT, cx)
                    });
                }
            }
            Place::In(yaar_review_pane::KEY_CONTEXT) => {
                self.click_into(cx, "review-bar");
                wait_until(cx, "the focus in the review pane", |cx| {
                    self.focus_is_in(yaar_review_pane::KEY_CONTEXT, cx)
                        && !self.focus_is_in(PROGRAM_CONTEXT, cx)
                });
            }
            Place::In(yaar_review_pane::HISTORY_CONTEXT) => {
                if !self.focus_is_in(yaar_review_pane::HISTORY_CONTEXT, cx) {
                    self.show_changes(cx);
                    wait_until(cx, "the tree of Changes", |cx| {
                        cx.debug_bounds("review-history-uncommitted").is_some()
                    });
                    self.click_into(cx, "review-row-0");
                    cx.simulate_keystrokes("tab");
                    wait_until(cx, "the focus in the tree of Changes", |cx| {
                        self.focus_is_in(yaar_review_pane::HISTORY_CONTEXT, cx)
                    });
                }
            }
            Place::In(PROGRAM_CONTEXT) => {
                let open = |window: &Self, cx: &mut VisualTestContext| {
                    window.terminals.read_with(&*cx, |pane, _| {
                        pane.shown()
                            .iter()
                            .any(|terminal| pane.viewed_attachment(terminal).is_some())
                    })
                };
                if open(self, cx) {
                    cx.simulate_keystrokes(&key_of("View Attachment"));
                    wait_until(cx, "the view of the attachment to close", |cx| {
                        !open(self, cx)
                    });
                }
                self.click_into(cx, "terminal-slot-0");
                wait_until(
                    cx,
                    &format!("the focus in the terminal for {}", control.title),
                    |cx| self.focus_is_in(PROGRAM_CONTEXT, cx),
                );
            }
            Place::In(yaar_review_pane::REFERENCES_CONTEXT) => self.open_the_list_of_references(cx),
            Place::In(other) => panic!(
                "{} works in the context {other:?}, and the walk does not know where that is",
                control.title
            ),
        }
    }

    /// The app must run while this waits: answers are taken on the thread that runs it.
    fn wait_for_the_request(
        &mut self,
        from: usize,
        wanted: impl Fn(&Request) -> bool,
        cx: &mut VisualTestContext,
    ) -> Request {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            let requests = self.tap.requests();
            if let Some(found) = requests[from..].iter().find(|request| wanted(request)) {
                return found.clone();
            }
            assert!(
                Instant::now() < deadline,
                "the request did not pass within {DEADLINE:?}; those looked at: {:?}",
                &requests[from..]
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }

    fn wait_for_the_echoes(&mut self, setting: &Setting, cx: &mut VisualTestContext) {
        wait_until(
            cx,
            &format!("the Keeper's word on every set of {setting:?}'s kind"),
            |_| {
                let sent = self
                    .tap
                    .requests()
                    .iter()
                    .filter(|request| {
                        matches!(request, Request::SetSetting { setting: set }
                            if set.same_thing_as(setting))
                    })
                    .count();
                let heard = ECHOES.with(|echoes| {
                    echoes
                        .borrow()
                        .iter()
                        .filter(|told| told.same_thing_as(setting))
                        .count()
                });
                heard >= sent
            },
        );
    }

    fn settings_are_open(&self, cx: &mut VisualTestContext) -> bool {
        self.frame
            .read_with(&*cx, |frame, _| frame.settings_screen().is_some())
    }

    fn open_the_settings(&mut self, cx: &mut VisualTestContext) {
        if !self.settings_are_open(cx) {
            self.leave_the_terminal_off_macos(cx);
            cx.simulate_keystrokes(&key_of("Settings…"));
        }
        wait_until(cx, "the settings screen to have the focus", |cx| {
            self.settings_are_open(cx) && self.focus_is_in(SETTINGS_CONTEXT, cx)
        });
    }

    fn close_the_settings(&mut self, cx: &mut VisualTestContext) {
        if self.settings_are_open(cx) {
            cx.simulate_keystrokes(&key_of("Close Settings"));
        }
        wait_until(cx, "the settings screen to close", |cx| {
            !self.settings_are_open(cx)
        });
    }

    fn put_the_cursor_on(&mut self, row: &SettingRow, cx: &mut VisualTestContext) {
        let screen = self
            .frame
            .read_with(&*cx, |frame, _| frame.settings_screen().cloned())
            .expect("the screen is open");
        let (next, previous) = (key_of("Next Setting"), key_of("Previous Setting"));
        loop {
            let (at, cursor) = screen.read_with(&*cx, |screen, cx| {
                let at = screen
                    .rows(cx)
                    .iter()
                    .position(|shown| shown.row == *row)
                    .unwrap_or_else(|| panic!("the screen has no row {row:?}"));
                (at, screen.cursor())
            });
            if cursor == at {
                return;
            }
            cx.simulate_keystrokes(if cursor < at { &next } else { &previous });
        }
    }

    fn press(&mut self, control: &Control, cx: &mut VisualTestContext) {
        assert!(
            !control.keystroke.is_empty(),
            "{} has no key",
            control.title
        );
        if control.place != Place::In(SETTINGS_CONTEXT) {
            self.close_the_settings(cx);
        }
        self.leave_what_the_navigator_opened(cx);
        self.focus_where_the_key_works(control, cx);
        self.ran.borrow_mut().clear();
        cx.simulate_keystrokes(&control.key());
        assert_eq!(
            self.ran.take(),
            [(control.action)().name()],
            "what the key {} of {} ran",
            control.key(),
            control.title
        );
        self.wait_for_the_navigator(control, cx);
    }

    fn leave_what_the_navigator_opened(&mut self, cx: &mut VisualTestContext) {
        if self
            .review
            .read_with(&*cx, |pane, _| pane.file_view().is_some())
        {
            self.click_into(cx, "review-file-back");
            wait_until(cx, "the file view to close", |cx| {
                self.review
                    .read_with(cx, |pane, _| pane.file_view().is_none())
            });
        }
        if self
            .review
            .read_with(&*cx, |pane, _| pane.references().is_some())
        {
            cx.simulate_keystrokes("escape");
            wait_until(cx, "the list of references to close", |cx| {
                self.review
                    .read_with(cx, |pane, _| pane.references().is_none())
            });
        }
    }

    fn wait_for_the_navigator(&mut self, control: &Control, cx: &mut VisualTestContext) {
        wait_until(cx, "the Navigator's answers to the pane's asks", |cx| {
            self.review.read_with(cx, |pane, _| {
                pane.hover().is_none_or(|hover| !hover.waiting)
                    && pane
                        .references()
                        .is_none_or(|references| !references.waiting)
            })
        });
        if (control.action)().name() == yaar_review_pane::GoToDefinition.name() {
            wait_until(cx, "the answer to the definition", |cx| {
                self.review.read_with(cx, |pane, _| {
                    pane.file_view().is_some()
                        || pane.notice().is_some()
                        || pane
                            .references()
                            .is_some_and(|references| !references.waiting)
                })
            });
        }
        if (control.action)().name() == yaar_review_pane::OpenReference.name() {
            wait_until(cx, "the file view of the reference", |cx| {
                self.review
                    .read_with(cx, |pane, _| pane.file_view().is_some())
            });
        }
    }

    fn repository_of_the_workspace_on_screen(
        &self,
        cx: &mut VisualTestContext,
    ) -> yaar_link::RepositoryId {
        let on_screen = self
            .on_screen(cx)
            .expect("a workspace is on screen before the walk asks for its repository");
        self.models.workspaces.read_with(cx, |list, _| {
            list.repositories()
                .iter()
                .find(|repository| {
                    repository
                        .workspaces
                        .iter()
                        .any(|workspace| workspace.id == on_screen.id)
                })
                .map(|repository| repository.id.clone())
                .expect("the list holds the repository of the workspace on screen")
        })
    }

    fn put_the_reviewer_on_an_added_line(&mut self, cx: &mut VisualTestContext) {
        self.show_changes(cx);
        self.click_into_the_review_pane(cx);
        let mut row = None;
        wait_until(cx, "the added line of the notes drawn", |cx| {
            let rows = self.review.read_with(cx, |pane, _| pane.rows().len());
            row = (0..rows).find(|row| {
                cx.debug_bounds(leak(format!("review-text-{row}-new:2")))
                    .is_some()
            });
            row.is_some()
        });
        self.click_into(cx, leak(format!("review-row-{}", row.expect("a row"))));
    }

    /// An earlier comparison's answer can take the rows and the reviewer's place away when it
    /// arrives, so the whole opening is repeated until the list is there.
    fn open_the_list_of_references(&mut self, cx: &mut VisualTestContext) {
        let deadline = Instant::now() + DEADLINE;
        let mut tried: Vec<String> = Vec::new();
        loop {
            match self.try_to_open_the_list_of_references(cx) {
                Ok(()) => break,
                Err(why) => {
                    eprintln!(
                        "the opening of the list of references failed on try {}: {why}",
                        tried.len() + 1
                    );
                    tried.push(why);
                }
            }
            assert!(
                Instant::now() < deadline,
                "waited in vain for the list of references; each try ended as follows: {tried:#?}; \
                 at the deadline {}",
                self.what_the_review_pane_holds(cx)
            );
        }
        assert!(
            self.focus_is_in(yaar_review_pane::REFERENCES_CONTEXT, cx),
            "the list of references took no focus"
        );
    }

    fn try_to_open_the_list_of_references(
        &mut self,
        cx: &mut VisualTestContext,
    ) -> Result<(), String> {
        self.show_changes(cx);
        self.click_into_the_review_pane(cx);
        let mut row = None;
        wait_until_telling(
            cx,
            "a line of the notes with a name drawn",
            |cx| {
                let rows = self.review.read_with(cx, |pane, _| pane.rows().len());
                row = (0..rows).find_map(|row| {
                    ["new", "old"].into_iter().find_map(|side| {
                        cx.debug_bounds(leak(format!("review-text-{row}-{side}:three")))
                            .map(|_| (row, side))
                    })
                });
                row.is_some()
            },
            |cx| Some(self.what_the_review_pane_holds(cx)),
        );
        let (row, side) = row.expect("a row");
        let selector = leak(format!("review-text-{row}-{side}:three"));
        let bounds = cx.debug_bounds(selector).expect("the text is drawn");
        cx.simulate_click(
            gpui_kit::point(bounds.left() + gpui_kit::px(2.), bounds.center().y),
            Modifiers::none(),
        );
        wait_until(
            cx,
            "the focus in the review pane after the click on the name",
            |cx| {
                self.focus_is_in(yaar_review_pane::KEY_CONTEXT, cx)
                    && !self.focus_is_in(PROGRAM_CONTEXT, cx)
            },
        );
        let deadline = Instant::now() + DEADLINE;
        cx.simulate_keystrokes(&key_of("Find References"));
        let held = self.review.read_with(cx, |pane, _| {
            (
                pane.references().map(|references| references.waiting),
                pane.notice(),
            )
        });
        if let (None, Some(notice)) = &held {
            return Err(format!(
                "the key of Find References asked nothing and said {notice:?}"
            ));
        }
        loop {
            cx.run_until_parked();
            let (list, notice) = self.review.read_with(cx, |pane, _| {
                (
                    pane.references().map(|references| references.waiting),
                    pane.notice(),
                )
            });
            match list {
                Some(false) => return Ok(()),
                None if notice.is_some() => {
                    return Err(format!(
                        "the answer left no list and the pane said {notice:?}"
                    ));
                }
                _ => {}
            }
            assert!(
                Instant::now() < deadline,
                "waited in vain for the list of references: the pane holds {list:?} (Some(true): \
                 the Navigator never answered; None: no list was asked and nothing was said); {}",
                self.what_the_review_pane_holds(cx)
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }

    fn assert_asks_nothing(&self, control: &Control, cx: &mut VisualTestContext) {
        assert!(
            !cx.did_prompt_for_paths(),
            "the key {} of {} says it asks for nothing and opened the dialog for a folder",
            control.key(),
            control.title
        );
        assert!(
            cx.debug_bounds("frame-base-field").is_none(),
            "the key {} of {} says it asks for nothing and opened the field for a line of text",
            control.key(),
            control.title
        );
        for field in ["frame-worktree-field", "settings-folder-field"] {
            assert!(
                cx.debug_bounds(field).is_none(),
                "the key {} of {} says it asks for nothing and opened the field {field}",
                control.key(),
                control.title
            );
        }
        assert!(
            cx.debug_bounds("review-commit-field").is_none(),
            "the key {} of {} says it asks for nothing and opened the field for a commit message",
            control.key(),
            control.title
        );
    }

    fn give(&mut self, control: &Control, answer: &Answer, cx: &mut VisualTestContext) {
        match (control.asks, answer) {
            (Asks::Nothing, Answer::Nothing) => self.assert_asks_nothing(control, cx),
            (Asks::Folder, Answer::Folder(folder)) => {
                assert!(
                    cx.did_prompt_for_paths(),
                    "the key {} of {} opens the dialog for a folder",
                    control.key(),
                    control.title
                );
                cx.simulate_path_prompt_response(|_| Some(vec![folder.clone()]));
                cx.run_until_parked();
            }
            (Asks::Text, Answer::Text(text)) => {
                cx.simulate_input(text);
                cx.simulate_keystrokes("enter");
            }
            (asks, answer) => panic!(
                "{} asks for {asks:?}, and the walk has {answer:?} to give",
                control.title
            ),
        }
    }

    fn walk(&mut self, kind: RequestKind, cx: &mut VisualTestContext) {
        self.walk_the_controls_titled(kind, |_| true, cx);
    }

    fn walk_the_controls_titled(
        &mut self,
        kind: RequestKind,
        chosen: impl Fn(&str) -> bool,
        cx: &mut VisualTestContext,
    ) {
        let controls = controls_that_send(kind);
        assert!(
            !controls.is_empty(),
            "no control of the Window sends {kind:?}: a user has no way to start it"
        );
        for control in controls.into_iter().filter(|control| chosen(control.title)) {
            let mut back_to: Option<WorkspaceId> = None;
            let mut launched_after: Option<u32> = None;
            let mut untouched: Vec<(Reads, Vec<String>)> = Vec::new();
            let (answer, wanted): (Answer, Wanted) = match kind {
                RequestKind::OpenFolder => {
                    let folder = self.new_repository();
                    let request = Request::OpenFolder {
                        path: folder.clone(),
                    };
                    (
                        Answer::Folder(folder),
                        Box::new(move |passed| *passed == request),
                    )
                }
                RequestKind::SetBase => {
                    let (on_screen, branch) = self.new_branch_on_screen(cx);
                    let request = Request::SetBase {
                        workspace: on_screen.id,
                        base: branch.clone(),
                    };
                    (
                        Answer::Text(branch),
                        Box::new(move |passed| *passed == request),
                    )
                }
                RequestKind::OpenTerminal => match control.title {
                    "New Terminal" => {
                        let on_screen = self
                            .on_screen(cx)
                            .expect("a workspace is on screen before the walk opens a terminal");
                        (
                            Answer::Nothing,
                            Box::new(move |passed| {
                                matches!(
                                    passed,
                                    Request::OpenTerminal {
                                        workspace,
                                        program: Program::Shell,
                                        ..
                                    } if *workspace == on_screen.id
                                )
                            }),
                        )
                    }
                    "New Agent Terminal" | "Resume Agent" => {
                        let cut_off = self
                            .cut_off
                            .as_ref()
                            .expect("the walk opened the Window after a restart cut a session off");
                        let (workspace, session) =
                            (cut_off.workspace.clone(), cut_off.session.clone());
                        back_to = self.bring_on_screen(&workspace, cx);
                        launched_after = Some(self.world.agent_log("default").pid);
                        let program = if control.title == "Resume Agent" {
                            Program::Resume { session }
                        } else {
                            let chosen = self
                                .terminals
                                .read_with(cx, |pane, cx| pane.chosen_cli(cx))
                                .expect("the Keeper names the CLIs the launchers start");
                            Program::Agent {
                                agent: chosen.agent,
                            }
                        };
                        let request = move |passed: &Request| {
                            matches!(passed, Request::OpenTerminal { workspace: of, program: opened, .. }
                                if *of == workspace && *opened == program)
                        };
                        (Answer::Nothing, Box::new(request))
                    }
                    other => {
                        panic!("the walk does not know the control {other} that opens a terminal")
                    }
                },
                RequestKind::DeleteDraft => {
                    let (review, comment) = self.write_a_draft(cx);
                    let request = Request::DeleteDraft { review, comment };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::CopyReview => {
                    let (review, _) = self.write_a_draft(cx);
                    let request = Request::CopyReview { review };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::MarkReviewed => {
                    let request = Request::MarkReviewed {
                        review: self.review_shown(cx),
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::SetViewed => {
                    let on_screen = self
                        .on_screen(cx)
                        .expect("a workspace is on screen before the walk marks a file viewed");
                    let request = Request::SetViewed {
                        workspace: on_screen.id,
                        path: PathBuf::from("notes.txt"),
                        viewed: true,
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::Commit => {
                    let on_screen = self
                        .on_screen(cx)
                        .expect("a workspace is on screen before the walk commits");
                    let request = Request::Commit {
                        workspace: on_screen.id,
                        message: COMMIT_MESSAGE.to_owned(),
                    };
                    (
                        Answer::Text(COMMIT_MESSAGE.to_owned()),
                        Box::new(move |passed| *passed == request),
                    )
                }
                RequestKind::SetSetting
                    if matches!(
                        control.title,
                        "Run Language Servers of the Repository"
                            | "Keep Language Servers of the Repository Out"
                    ) =>
                {
                    back_to = self.on_screen(cx).map(|workspace| workspace.id);
                    let folder = self.new_repository();
                    self.show_changes(cx);
                    self.open_this_folder(&folder, cx);
                    let repository = self.repository_of_the_workspace_on_screen(cx);
                    self.put_the_reviewer_on_an_added_line(cx);
                    wait_until(cx, "the question on the line of the pane", |cx| {
                        cx.debug_bounds("review-servers-yes").is_some()
                            && cx.debug_bounds("review-servers-no").is_some()
                    });
                    let allowed = control.title == "Run Language Servers of the Repository";
                    let request = Request::SetSetting {
                        setting: Setting::ServersInRepository {
                            repository,
                            allowed,
                        },
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::SetSetting if control.title == "Move Panes" => {
                    let next = self.frame.read_with(cx, |frame, _| frame.beside()).next();
                    let request = Request::SetSetting {
                        setting: Setting::TerminalPlace(place_of(next)),
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::SetSetting
                    if matches!(control.title, "Next Value" | "Previous Value") =>
                {
                    self.close_the_settings(cx);
                    self.open_the_settings(cx);
                    self.put_the_cursor_on(&SettingRow::TurnEnd, cx);
                    let now = self
                        .models
                        .settings
                        .read_with(cx, |settings, _| settings.turn_end());
                    let to = if control.title == "Next Value" {
                        (TurnEndSelects::WhatChanged, TurnEndSelects::KeepSelection)
                    } else {
                        (TurnEndSelects::KeepSelection, TurnEndSelects::WhatChanged)
                    };
                    assert_eq!(
                        now, to.0,
                        "{} moves what a turn end selects from {:?}; the walk met it as {now:?}",
                        control.title, to.0
                    );
                    let request = Request::SetSetting {
                        setting: Setting::TurnEnd(to.1),
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::SetSetting if control.title == "Keep Selection" => {
                    let flipped = match self
                        .models
                        .settings
                        .read_with(cx, |settings, _| settings.turn_end())
                    {
                        TurnEndSelects::WhatChanged => TurnEndSelects::KeepSelection,
                        TurnEndSelects::KeepSelection => TurnEndSelects::WhatChanged,
                    };
                    let request = Request::SetSetting {
                        setting: Setting::TurnEnd(flipped),
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::SetSetting if control.title == "Timeline or Live Diff" => {
                    self.show_flow(cx);
                    let on_screen = self
                        .on_screen(cx)
                        .expect("a workspace is on screen before the walk sets a form of Flow");
                    let request = Request::SetSetting {
                        setting: Setting::FlowForm {
                            workspace: on_screen.id,
                            form: FlowForm::LiveDiff,
                        },
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::SetSetting => {
                    panic!(
                        "the walk has no step for the control {}, which sends SetSetting",
                        control.title
                    )
                }
                RequestKind::ListWorkspaces
                | RequestKind::GetChanges
                | RequestKind::ReadFile
                | RequestKind::ResizeTerminal
                | RequestKind::GetReviews
                | RequestKind::SaveDraft
                | RequestKind::ListAgents
                | RequestKind::GetHistory
                | RequestKind::GetUncommitted
                | RequestKind::CompareVersions
                | RequestKind::ReadVersions
                | RequestKind::GetTurn
                | RequestKind::ShowWorkspace
                | RequestKind::ListTerminals
                | RequestKind::AttachTerminal
                | RequestKind::GetSettings
                | RequestKind::GetAttachment
                | RequestKind::ListClis
                | RequestKind::CompareForReview
                | RequestKind::ListConsents
                | RequestKind::ExportVersion
                | RequestKind::GetStateSizes => {
                    panic!("the Window starts {kind:?} by itself: no user does")
                }
                RequestKind::Report => {
                    panic!("a piece of an agent CLI starts {kind:?}: no user does")
                }
                RequestKind::CreateWorktree => {
                    let on_screen = self
                        .on_screen(cx)
                        .expect("a workspace is on screen before the walk makes a worktree");
                    let Some(Head::Branch(from)) = on_screen.head.clone() else {
                        panic!("the workspace on screen has no branch checked out: {on_screen:?}")
                    };
                    let repository = self.repository_of_the_workspace_on_screen(cx);
                    let agent = self
                        .terminals
                        .read_with(cx, |pane, cx| pane.chosen_cli(cx))
                        .expect("the Keeper names the CLIs the launchers start")
                        .agent;
                    // The world's agent is the stand-in as Claude Code, whatever the pane reads.
                    assert_eq!(agent, "claude", "the CLI the walk chooses");
                    let branch = format!("made-{}", self.next());
                    back_to = Some(on_screen.id);
                    launched_after = Some(self.world.agent_log("default").pid);
                    (
                        Answer::Text(branch.clone()),
                        Box::new(move |passed| {
                            matches!(passed, Request::CreateWorktree {
                                repository: of, from: made_from, branch: named, agent: with, ..
                            } if *of == repository && *made_from == from && *named == branch && *with == agent)
                        }),
                    )
                }
                RequestKind::CleanUpStates => {
                    let home = self.world.home();
                    untouched = self
                        .repositories
                        .iter()
                        .map(|repository| {
                            let planted = plant(&home, repository);
                            (reads(&home, repository), planted)
                        })
                        .collect();
                    (
                        Answer::Nothing,
                        Box::new(|passed| matches!(passed, Request::CleanUpStates)),
                    )
                }
                RequestKind::AnswerConsent => {
                    let answer = if control.title == "Install Agent Extensions" {
                        ConsentAnswer::Yes
                    } else {
                        ConsentAnswer::No
                    };
                    let request = Request::AnswerConsent {
                        agent: "pi".to_owned(),
                        answer,
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::AttachReview => {
                    let (review, thread) = self.agent_with_a_thread(cx);
                    let request = Request::AttachReview {
                        review,
                        thread: (control.title == "Attach Thread").then_some(thread),
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::DetachReview => {
                    let (review, _) = self.agent_with_a_thread(cx);
                    self.make_sure_it_is_attached(cx);
                    let request = Request::DetachReview { review };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::ResolveThread | RequestKind::ReopenThread => {
                    let (review, thread) = self.agent_with_a_thread(cx);
                    let request = if kind == RequestKind::ResolveThread {
                        Request::ResolveThread { review, thread }
                    } else {
                        Request::ReopenThread { review, thread }
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::ResolveAddressed => {
                    let (review, _) = self.agent_with_a_thread(cx);
                    let request = Request::ResolveAddressed { review };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::CloseTerminal => {
                    let before: Vec<_> = self.terminals.read_with(cx, |pane, _| pane.terminals());
                    self.press(&Self::control_titled("New Terminal"), cx);
                    let mut opened = None;
                    wait_until(cx, "the fresh terminal to close", |cx| {
                        opened = self.terminals.read_with(cx, |pane, _| {
                            pane.focused().filter(|id| !before.contains(id))
                        });
                        opened.is_some()
                    });
                    let terminal = opened.expect("a newly opened terminal is focused");
                    let request = Request::CloseTerminal { terminal };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
            };

            let from = self.tap.requests().len();
            self.press(&control, cx);
            self.give(&control, &answer, cx);
            let passed = self.wait_for_the_request(from, |passed| wanted(passed), cx);
            self.settle(kind, &passed, from, cx);
            let home = self.world.home();
            for (repository, (before, planted)) in self.repositories.iter().zip(&untouched) {
                assert_eq!(
                    &reads(&home, repository),
                    before,
                    "the clean-up wrote into the repository {}",
                    repository.path().display()
                );
                still_there(repository, planted);
            }
            if let Some(before) = launched_after {
                self.end_the_stand_in_started_after(before);
            }
            if let Some(workspace) = back_to {
                self.bring_on_screen(&workspace, cx);
            }

            if let Answer::Folder(folder) = &answer {
                self.wait_until_on_screen(folder, cx);
            }
        }
    }

    fn bring_on_screen(
        &mut self,
        workspace: &WorkspaceId,
        cx: &mut VisualTestContext,
    ) -> Option<WorkspaceId> {
        let before = self.on_screen(cx).map(|shown| shown.id);
        if before.as_ref() == Some(workspace) {
            return None;
        }
        let row = self.point_in(&format!("sidebar-workspace-{workspace}"), cx);
        cx.simulate_click(row, Modifiers::none());
        wait_until(cx, "the workspace on screen", |cx| {
            self.on_screen(cx)
                .is_some_and(|shown| shown.id == *workspace)
        });
        before
    }

    fn end_the_stand_in_started_after(&mut self, before: u32) {
        let world = &self.world;
        assert!(
            yaar_system_tests::wait_until(DEADLINE, || {
                let started = world.agent_log("default").pid;
                started != 0 && started != before
            }),
            "no stand-in started after the control opened its terminal"
        );
        let pid = self.world.agent_log("default").pid;
        let gate = gate_of(&self.world);
        std::fs::write(&gate, "").expect("make the gate");
        assert!(
            yaar_system_tests::wait_until(DEADLINE, || !is_alive(pid)),
            "the stand-in {pid} did not end at the gate"
        );
        std::fs::remove_file(&gate).expect("take the gate away");
    }

    fn control_titled(title: &str) -> Control {
        yaar_window::controls()
            .into_iter()
            .find(|control| control.title == title)
            .unwrap_or_else(|| panic!("no control is titled {title:?}"))
    }

    fn agent_with_a_thread(&mut self, cx: &mut VisualTestContext) -> (ReviewId, ThreadId) {
        if self.agent.is_none() {
            self.start_an_agent(cx);
        }
        if self.agent().thread.is_none() {
            // The kinds before wrote a thread on the first line.
            let (review, comment) = self.write_a_draft_lower(1, cx);
            assert_eq!(
                review,
                self.agent().review,
                "the draft is in the agent's review"
            );
            let thread = self.review.read_with(cx, |pane, _| {
                pane.review()
                    .and_then(|review| {
                        review
                            .threads
                            .iter()
                            .find(|thread| thread.comments.iter().any(|c| c.id == comment))
                    })
                    .map(|thread| thread.id.clone())
            });
            let thread = thread.expect("the review holds the thread of the draft");
            self.agent.as_mut().expect("started").thread = Some(thread);
        }
        let agent = self.agent();
        (
            agent.review.clone(),
            agent.thread.clone().expect("the thread was written"),
        )
    }

    fn make_sure_it_is_attached(&mut self, cx: &mut VisualTestContext) {
        let attached = self
            .review
            .read_with(cx, |pane, _| pane.review().and_then(|r| r.attached.clone()));
        if attached.is_none() {
            self.press(&Self::control_titled("Attach Review"), cx);
            self.wait_for_attachment(Some(Attachment::Review), cx);
        }
    }

    fn put_the_reviewer_on_the_thread(&mut self, cx: &mut VisualTestContext) {
        let thread = self
            .agent()
            .thread
            .clone()
            .expect("the walk wrote a thread");
        self.show_changes(cx);
        self.click_into_the_review_pane(cx);
        let rows_of = |window: &Self, cx: &mut VisualTestContext| {
            window.review.read_with(cx, |pane, _| {
                let of_thread = pane.rows().iter().position(
                    |row| matches!(row, PaneRow::Thread { thread: held } if *held == thread),
                );
                (pane.position_row(), of_thread)
            })
        };
        wait_until(cx, "the thread's row and the reviewer on a line", |cx| {
            matches!(rows_of(self, cx), (Some(_), Some(_)))
        });
        let (line_down, line_up) = (key_of("Line Down"), key_of("Line Up"));
        let rows = self.review.read_with(&*cx, |pane, _| pane.rows().len());
        for _ in 0..rows {
            let (Some(on), Some(thread_row)) = rows_of(self, cx) else {
                break;
            };
            if on + 1 == thread_row {
                break;
            }
            cx.simulate_keystrokes(if on + 1 < thread_row {
                &line_down
            } else {
                &line_up
            });
            cx.run_until_parked();
        }
        wait_until(
            cx,
            "the reviewer on the line of the thread",
            |cx| matches!(rows_of(self, cx), (Some(on), Some(thread_row)) if on + 1 == thread_row),
        );
    }

    fn end_the_agent(&mut self) {
        std::fs::write(gate_of(&self.world), "").expect("make the gate");
        let world = &self.world;
        assert!(
            yaar_system_tests::wait_until(DEADLINE, || world
                .agent_log("default")
                .did
                .iter()
                .any(|did| matches!(did, Did::Exited { .. }))),
            "the agent did not end at the gate"
        );
    }

    fn walk_the_stop(&mut self, cx: &mut VisualTestContext) {
        let controls = controls_that_stop();
        assert!(
            !controls.is_empty(),
            "no control of the Window sends the stop-everything frame: a user has no way to start it"
        );
        // Listed while the Keeper runs: once it is gone its programs belong to the system.
        let keeper = self.world.keeper_pid().expect("the world's Keeper runs");
        let programs = descendants_of(keeper);
        assert!(
            !programs.is_empty(),
            "the walk opened a terminal, and the Keeper holds its program"
        );
        for control in controls {
            self.press(&control, cx);
            self.give(&control, &Answer::Nothing, cx);
            assert!(
                self.tap.wait_for_stop(DEADLINE),
                "the key {} of {} did not send the stop-everything frame",
                control.key(),
                control.title
            );
            let world = &mut self.world;
            let gone = yaar_system_tests::wait_until(DEADLINE, || world.keeper_pid().is_none());
            assert!(
                gone,
                "the Keeper still runs after the key {} of {}\n{}",
                control.key(),
                control.title,
                self.keeper_that_stays()
            );
        }
        assert!(
            yaar_system_tests::wait_until(DEADLINE, || programs.iter().all(|pid| !is_alive(*pid))),
            "the programs of the Keeper still run after the stop: {:?}",
            programs
                .iter()
                .filter(|pid| is_alive(**pid))
                .collect::<Vec<_>>()
        );
    }

    fn keeper_that_stays(&mut self) -> String {
        let errors = fs::read_to_string(self.world.home().with_file_name("keeper.err"))
            .unwrap_or_else(|error| format!("(unreadable: {error})"));
        let processes = match self.world.keeper_pid() {
            None => "the Keeper ended meanwhile".to_owned(),
            Some(keeper) => {
                let mut pids = vec![keeper];
                pids.extend(descendants_of(keeper));
                let listing = Command::new("ps")
                    .arg("-o")
                    .arg("pid,ppid,stat,etime,command")
                    .arg("-p")
                    .arg(
                        pids.iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    )
                    .output();
                match listing {
                    Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
                    Err(error) => format!("ps could not be run: {error}"),
                }
            }
        };
        format!("the Keeper's error output:\n{errors}\nthe processes under it:\n{processes}")
    }

    fn wait_until_on_screen(&mut self, folder: &Path, cx: &mut VisualTestContext) {
        wait_until(cx, "the opened folder on screen", |cx| {
            self.on_screen(cx)
                .is_some_and(|workspace| workspace.path == folder)
        });
    }

    fn set_base(&mut self, base: &str, cx: &mut VisualTestContext) -> Option<usize> {
        let control = controls_that_send(RequestKind::SetBase)
            .into_iter()
            .next()
            .expect("a control of the Window sends SetBase");
        let from = self.tap.requests().len();
        self.press(&control, cx);
        self.give(&control, &Answer::Text(base.to_owned()), cx);
        let sets_it = |request: &Request| match request {
            Request::SetBase { base: set, .. } => set == base,
            _ => false,
        };
        let deadline = Instant::now() + DEADLINE;
        let stopped = loop {
            cx.run_until_parked();
            if self.tap.requests()[from..].iter().any(sets_it) {
                break false;
            }
            if self.tap.wait_for_stop(Duration::ZERO) {
                break true;
            }
            assert!(
                Instant::now() < deadline,
                "neither the request that sets the base {base} nor the stop-everything frame \
                 passed the tap; the requests listed are {:?}",
                &self.tap.requests()[from..]
            );
            std::thread::park_timeout(Duration::from_millis(1));
        };
        if stopped || self.tap.wait_for_stop(Duration::ZERO) {
            return None;
        }
        let found = self.tap.requests()[from..]
            .iter()
            .position(sets_it)
            .expect("the tap lists the request it waited for");
        let refused = Refusal::UnknownRevision {
            revision: base.to_owned(),
        };
        let models = self.models.workspaces.clone();
        wait_until_telling(
            cx,
            &format!("the Keeper to refuse the base {base}"),
            |cx| models.read_with(cx, |list, _| list.refusal() == Some(&refused)),
            |cx| {
                Some(format!(
                    "the Frame held the refusal {:?}",
                    models.read_with(cx, |list, _| list.refusal().cloned())
                ))
            },
        );
        Some(from + found)
    }
}

#[gpui_kit::test]
fn every_request_a_user_starts_has_a_control_whose_key_sends_it(cx: &mut TestAppContext) {
    let (mut window, mut cx) = open_composed_for_the_walk(cx);

    // A workspace on screen first: without one there is nothing to set a base for.
    window.walk(RequestKind::OpenFolder, &mut cx);

    for kind in RequestKind::started_by_user() {
        window.walk(kind, &mut cx);
    }
    window.end_the_agent();

    // Last: the stop frame ends the Keeper.
    window.walk_the_stop(&mut cx);
    window.end_the_navigator(&mut cx);
}

fn walk_the_silent_keys(
    window: &mut Composed,
    cx: &mut VisualTestContext,
    mut ready: impl FnMut(&mut Composed, &mut VisualTestContext),
) {
    // The test platform's hide, hide-others and minimise calls are `unimplemented!()`. A handler
    // registered after the app's runs first and stops the action, doing nothing.
    cx.update(|_, app| {
        app.on_action(|_: &yaar_frame::HideApp, _| {});
        app.on_action(|_: &yaar_frame::HideOtherApps, _| {});
        app.on_action(|_: &yaar_frame::Minimize, _| {});
    });
    let mut silent: Vec<Control> = yaar_window::controls()
        .into_iter()
        .filter(|control| control.sends.is_none())
        .collect();
    // Quit and Close Window come last; the test platform does not end the app on Quit, so the
    // window remains to close the step.
    let (quit, close) = (yaar_window::Quit.name(), CloseWindow.name());
    let ends_the_app = |control: &Control| {
        let name = (control.action)().name();
        name == quit || name == close
    };
    silent.sort_by_key(|control| ends_the_app(control));
    assert!(
        silent.iter().rev().take(2).all(ends_the_app)
            && silent
                .iter()
                .filter(|control| ends_the_app(control))
                .count()
                == 2,
        "Quit and Close Window are the last two keys of the walk"
    );

    // Resume Follow acts in Flow and must send nothing in Changes either, so it is pressed twice,
    // the second time with Changes shown.
    let resume = yaar_review_pane::ResumeFollow.name();
    let mut steps: Vec<(Control, bool)> = Vec::new();
    for control in silent {
        steps.push((control, false));
        if (control.action)().name() == resume {
            steps.push((control, true));
        }
    }
    // An absence is shown by the closing request: what a key sent lies before it on the one
    // connection. A request sent only after the user's input is not seen, so each key must also
    // leave the folder dialog and the base and commit fields closed.
    for (step, (control, in_changes)) in steps.into_iter().enumerate() {
        assert_eq!(
            control.asks,
            Asks::Nothing,
            "{} asks the user for something, and the walk does not know what to give",
            control.title
        );
        ready(window, cx);
        let before = window.tap.requests().len();
        if in_changes {
            window.show_changes(cx);
        }
        // Close Window closes the active window, and closing the only one would end the app: a
        // spare one is made active.
        let spare = (control.action)().name() == CloseWindow.name();
        let spare = spare.then(|| open_a_spare_window(cx));
        window.press(&control, cx);
        if let Some(spare) = spare {
            wait_until(cx, "the key to close the active window", |cx| {
                cx.update(|_, app| !app.windows().contains(&spare))
            });
            assert_eq!(
                cx.update(|_, app| app.windows().len()),
                1,
                "the key closed more than the window that was active"
            );
            cx.update(|window, _| window.activate_window());
        }
        // Before the step is closed: the closing types into the base field and would fill a field
        // this key had opened.
        window.assert_asks_nothing(&control, cx);
        // The keys of Comment, Edit Draft and Reply may open the box, which takes the focus: later
        // keys would be typed into it.
        if window
            .review
            .read_with(&*cx, |pane, _| pane.comment_box().is_some())
        {
            cx.simulate_keystrokes("escape");
            wait_until(cx, "the comment box to close", |cx| {
                window
                    .review
                    .read_with(cx, |pane, _| pane.comment_box().is_none())
            });
        }
        // The stop frame is no request: a key that sent it would end the Keeper and the closing
        // request would never pass.
        let Some(closed) = window.set_base(&format!("after-{}-{step}", control.key()), cx) else {
            panic!(
                "the key {} of {} names no message and sent the stop-everything frame",
                control.key(),
                control.title
            );
        };
        let base = format!("after-{}-{step}", control.key());
        let refusal = window
            .models
            .workspaces
            .read_with(&*cx, |list, _| list.refusal().cloned());
        assert_eq!(
            refusal,
            Some(Refusal::UnknownRevision { revision: base }),
            "the step of the key {} of {} went on before the Keeper had answered the request \
             that closes it",
            control.key(),
            control.title
        );
        let passed = window.tap.requests();
        for request in &passed[before..closed] {
            assert_ne!(
                request.kind().origin(),
                Origin::User,
                "the key {} of {} names no message and sent {request:?}",
                control.key(),
                control.title
            );
        }
    }

    assert_eq!(
        window.frame.read_with(&*cx, |frame, _| frame.panes().len()),
        yaar_window::panes().len()
    );
}

#[gpui_kit::test]
fn no_key_of_a_control_that_names_no_message_sends_what_a_user_starts(cx: &mut TestAppContext) {
    for control in yaar_window::controls() {
        match control.sends {
            Some(Sends::Request(kind)) => assert_eq!(
                kind.origin(),
                Origin::User,
                "{} says it sends {kind:?}",
                control.title
            ),
            Some(Sends::StopEverything) => {}
            None => {}
        }
    }

    let (mut window, mut cx) = open_composed(cx);
    // Two workspaces, so the keys that change the one on screen have one to change to.
    window.walk(RequestKind::OpenFolder, &mut cx);
    window.walk(RequestKind::OpenFolder, &mut cx);
    window.walk_the_controls_titled(
        RequestKind::OpenTerminal,
        |title| title == "New Terminal",
        &mut cx,
    );
    wait_until(&mut cx, "the terminal on screen", |cx| {
        cx.debug_bounds("terminal-slot-0").is_some()
    });
    walk_the_silent_keys(&mut window, &mut cx, |_, _| {});
    window.end_the_navigator(&mut cx);
}

#[gpui_kit::test]
fn no_key_of_a_control_that_names_no_message_sends_what_a_user_starts_where_an_agent_runs_with_a_review_attached(
    cx: &mut TestAppContext,
) {
    let (mut window, mut cx) = open_composed_with_an_agent(cx);
    // One workspace only: a key that changed the one on screen would take the agent from later
    // keys.
    window.walk(RequestKind::OpenFolder, &mut cx);
    window.agent_with_a_thread(&mut cx);
    window.make_sure_it_is_attached(&mut cx);

    walk_the_silent_keys(&mut window, &mut cx, |window, cx| {
        window.leave_what_the_navigator_opened(cx);
        let session = window.agent().session.clone();
        window.show_the_review_of(&session, cx);
        window.make_sure_it_is_attached(cx);
        window.put_the_reviewer_on_the_thread(cx);
    });
    window.end_the_agent();
    window.end_the_navigator(&mut cx);
}
