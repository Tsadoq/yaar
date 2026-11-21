//! Component test: the composed Window on GPUI's test platform, with every pane, and the
//! built Keeper program beneath it. No window is on any screen.
//!
//! The keys are pressed as a user presses them. What they set off is read on a tap
//! between the Window and the Keeper: the requests that reach the Keeper, and nothing
//! the Window keeps about itself.
//!
//! The tests walk the Window's table of controls against the Link's table of requests.
//! Who starts a kind of request is declared in the Link's table; a kind declared there as
//! the Window's own is outside the walk, and only a reader of that table can tell whether
//! it belongs there.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::{Action, Modifiers, TestAppContext, VisualTestContext};
use yaar_frame::{Asks, Control, PROGRAM_CONTEXT, Place, Sends};
use yaar_link::{
    Attachment, CommentId, ConsentAnswer, FlowForm, Origin, Program, Request, RequestKind,
    ReviewId, Setting, ThreadId, ThreadState, Workspace, WorkspaceId,
};
use yaar_review_pane::{PaneRow, View};
use yaar_system_tests::standin::{Did, Script};
use yaar_system_tests::{DEADLINE, Repo, World, descendants_of, is_alive};

mod common;

use common::{Composed, Found, RunningAgent, key_of, wait_until};

/// Which request a step of the walk waits for on the tap.
type Wanted = Box<dyn Fn(&Request) -> bool>;

/// The text of the draft the walk writes for the kinds that act on one.
const DRAFT: &str = "walked";

/// The message the walk commits with.
const COMMIT_MESSAGE: &str = "Walked commit";

/// What the walk has to give when a control asks the user for something.
#[derive(Debug)]
enum Answer {
    /// Nothing: the key alone sends the request.
    Nothing,
    /// The folder to choose in the dialog.
    Folder(PathBuf),
    /// The line to type into the field, before Enter.
    Text(String),
}

/// Opens the composed Window as the tests of its other rows do, in a world of its own with
/// a tap on its link, and checks that every pane of the Window's list is in it, as well as
/// in the table of controls. The Navigator runs in it ([`open_with_a_navigator`]).
fn open_composed(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    world.start_keeper();
    open_with_a_navigator(world, cx)
}

/// Opens the Window in `world`, whose Keeper runs, as the program opens it: with a Navigator
/// whose registry has one row, for the files `notes.txt` and its kind hold (the walk's
/// repositories hold `notes.txt` and no code) and for the stand-in's own `.sil`, whose server
/// is the project's own, `tools/standin-lsp` ([`new_repository`] puts one in each
/// repository the walk opens): the Navigator asks before it runs, which is the question the
/// walk answers. Checks that every pane of the Window's list is in it.
fn open_with_a_navigator(world: World, cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let navigator = common::navigator_for(
        &world,
        Found::InTheProject,
        &common::standin_row(Found::InTheProject, "notes.txt", &["txt"]),
    );
    check_the_panes(common::open_composed_with_a_navigator(
        world,
        cx,
        Vec::new(),
        navigator,
    ))
}

/// [`open_composed`] in a world whose agent CLI is the stand-in, with a script that runs and
/// does nothing until the test makes the file `gate` of the world: the session the walk
/// of the reviews starts, and ends at the gate.
fn open_composed_with_an_agent(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    world.agents_on_path();
    let script = Script::parse(&format!("hold {}\nexit", gate_of(&world).display()));
    world.agent_script("default", &script);
    world.start_keeper();
    open_with_a_navigator(world, cx)
}

/// [`open_composed_with_an_agent`] for the walk of every request, in a world where pi is
/// installed, so that the Window asks the question for leave, and where a restart cut off a
/// session of Claude Code in a workspace of its own, so that Resume Agent has a session to
/// bring back. The stand-ins that start after the restart hold at the gate of the world.
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

/// The file outside the worktrees that holds the stand-in until the test makes it.
fn gate_of(world: &World) -> PathBuf {
    world.home().with_file_name("gate")
}

/// A text that lives as long as the program, as the test platform takes a selector.
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

/// The controls of the Window that say they send the stop-everything frame.
fn controls_that_stop() -> Vec<Control> {
    yaar_window::controls()
        .into_iter()
        .filter(|control| control.sends == Some(Sends::StopEverything))
        .collect()
}

/// The controls of the Window that say they send a request of this kind.
fn controls_that_send(kind: RequestKind) -> Vec<Control> {
    yaar_window::controls()
        .into_iter()
        .filter(|control| control.sends == Some(Sends::Request(kind)))
        .collect()
}

impl Composed {
    /// The workspace on screen, once the list holds it.
    fn on_screen(&self, cx: &mut VisualTestContext) -> Option<Workspace> {
        self.models.workspaces.read_with(cx, |list, _| {
            list.selected()
                .and_then(|selected| list.workspace(selected))
                .cloned()
        })
    }

    /// A new repository of the world with something to review: on a branch that left
    /// `main` with one commit, and with one more change that is not committed. Returns
    /// its folder.
    fn new_repository(&mut self) -> PathBuf {
        let name = format!("walked-{}", self.next());
        let repository = common::repository_to_review(&self.world, &name);
        // A server of the project's own, which the Navigator asks about before it runs it.
        common::put_the_standin_in(&repository);
        let folder = repository.path().to_owned();
        self.repositories.push(repository);
        folder
    }

    /// The repository of the workspace on screen, which is one the walk opened.
    fn repository_on_screen(&self, cx: &mut VisualTestContext) -> &Repo {
        let on_screen = self
            .on_screen(cx)
            .expect("a workspace is on screen before the walk looks at its repository");
        self.repositories
            .iter()
            .find(|repository| repository.path() == on_screen.path)
            .expect("the workspace on screen is one the walk opened")
    }

    /// Makes a new branch in the repository of the workspace on screen, and returns that
    /// workspace with the name of the branch.
    fn new_branch_on_screen(&mut self, cx: &mut VisualTestContext) -> (Workspace, String) {
        let on_screen = self
            .on_screen(cx)
            .expect("a workspace is on screen before the walk sets a base");
        let branch = format!("base-{}", self.next());
        self.repository_on_screen(cx).git(&["branch", &branch]);
        (on_screen, branch)
    }

    /// Waits until the Keeper has done what the request of `kind` asked, as a user sees it:
    /// the copy on the clipboard, the mark on the review, the file shown viewed, the commit
    /// in the repository, the focus in the terminal that was opened. The walk ends when the
    /// last request passes the tap, and the world ends the Keeper with a kill: a Keeper
    /// still recording a state or running git for its answer leaves the folder of its copy
    /// of git's list of files in the system's temp folder, outside the world. The requests of the reviews are answered one after
    /// the other, so waiting for the last of them covers those before it.
    ///
    /// `passed` is the request the step waited for: for the kinds that act on a review it
    /// says what the step has to see done. `from` is the place on the tap where the step's own
    /// requests begin.
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
            // The pane takes the focus for a terminal when the Keeper's answer is taken, which
            // is after the request has passed the tap. A step that clicks into another pane
            // before that would have its click taken from it, and the keys after it typed
            // into the terminal.
            RequestKind::OpenTerminal => {
                wait_until(cx, "the new terminal to take the focus", |cx| {
                    self.focus_is_in("Program", cx)
                })
            }
            RequestKind::SetSetting => match passed {
                // The pane shows the form it set as soon as the key is pressed.
                Request::SetSetting {
                    setting: Setting::FlowForm { .. },
                } => wait_until(cx, "the live diff as the form of Flow", |cx| {
                    self.review
                        .read_with(cx, |pane, _| pane.flow_form() == FlowForm::LiveDiff)
                }),
                // The Keeper answers the question for the repository and tells it back; the
                // model of the Frame shows the answer, and the pane no longer asks.
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
                    // A yes lets the server run, which the Navigator heard of through the
                    // Frame: the line of the pane shows it ready.
                    if *allowed {
                        wait_until(cx, "the server of the repository ready", |cx| {
                            cx.debug_bounds("review-servers-state:ready").is_some()
                        });
                    }
                }
                other => panic!("the step of SetSetting waited for {other:?}"),
            },
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
            | RequestKind::ExportVersion => {
                panic!("the Window starts {kind:?} by itself: no user does")
            }
            RequestKind::Report => panic!("a piece of an agent CLI starts {kind:?}: no user does"),
            // The answer shows in pi's settings: a yes lists the extension of the app, and a
            // no takes it out again.
            RequestKind::AnswerConsent => {
                let Request::AnswerConsent { answer, .. } = passed else {
                    panic!("the step of AnswerConsent waited for {passed:?}")
                };
                let settings = self.world.pi_settings();
                let lists_it = move || {
                    std::fs::read_to_string(&settings).is_ok_and(|text| text.contains("yaar.ts"))
                };
                let wanted = *answer == ConsentAnswer::Yes;
                // And the Window holds the Keeper's answer: the question is gone from its
                // model, so that the next key asks the Keeper again what there is to answer.
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
                // The question is gone once answered, so the key that answers later asks the
                // Keeper which Adapters there are to answer for first.
                if !wanted {
                    // Only what this step's key sent counts: an earlier step's question does
                    // not say who asks now.
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
            // No thread of the walk is addressed, so the request changes nothing to wait
            // for: the Keeper answers it from the review it holds, with no git and no
            // file, so that nothing is left half done when the stop frame ends it.
            RequestKind::ResolveAddressed => {}
        }
    }

    /// Waits until the review of the agent shows what is attached to it as `attached` says,
    /// in the review pane and in the agents' list, which the agent's terminal reads.
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

    /// Waits until the thread of the walk is in `state` (`is` true) or not in it.
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

    /// The agent the walk started.
    fn agent(&self) -> &RunningAgent {
        self.agent.as_ref().expect("the walk started an agent")
    }

    /// The review the review pane shows, once it holds one.
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

    /// Writes a draft on the line the reviewer is on, as a user does: into the review pane,
    /// `c`, the text, Escape. A click on the first row puts the reviewer on the first line
    /// of the file, which is unchanged context: a comment may sit on any line. The pane
    /// shows Changes first, whichever view an earlier step left it in, since Flow draws no
    /// such row. Returns the review and the draft once the pane's model shows the draft
    /// with its whole text.
    fn write_a_draft(&mut self, cx: &mut VisualTestContext) -> (ReviewId, CommentId) {
        self.write_a_draft_lower(0, cx)
    }

    /// [`Composed::write_a_draft`] on the line `down` lines below the first: a line no
    /// earlier thread of the walk sits on, so that the keys that act on the thread on the
    /// reviewer's line find this one.
    fn write_a_draft_lower(
        &mut self,
        down: usize,
        cx: &mut VisualTestContext,
    ) -> (ReviewId, CommentId) {
        self.show_changes(cx);
        // The reviewer waits for the pane to show the reviews and a line to be on.
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

    /// Shows Changes in the review pane by a click on its name in the bar, which pauses no
    /// follow, and waits until it is on screen with its first row drawn.
    fn show_changes(&mut self, cx: &mut VisualTestContext) {
        self.click_into(cx, "review-view-changes");
        wait_until(cx, "Changes on screen", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.view() == View::Changes)
                && cx.debug_bounds("review-row-0").is_some()
        });
    }

    /// Shows Flow in the review pane by a click on its name in the bar, and waits until it
    /// is on screen.
    fn show_flow(&mut self, cx: &mut VisualTestContext) {
        self.click_into(cx, "review-view-flow");
        wait_until(cx, "Flow on screen", |cx| {
            self.review
                .read_with(cx, |pane, _| pane.view() == View::Flow)
        });
    }

    /// Puts the focus where the key of `control` works. A key that works anywhere is
    /// pressed where the focus happens to be, unless that is inside a terminal: off macOS
    /// such a key does nothing there, so the focus is first put in the review pane, which
    /// on macOS does no harm. New Terminal, Next Terminal and Previous Terminal move the
    /// focus into a terminal, and so does a click into one, so this is looked at before
    /// every key.
    fn focus_where_the_key_works(&mut self, control: &Control, cx: &mut VisualTestContext) {
        match control.place {
            // A kept key works wherever the focus is.
            Place::Kept => {}
            Place::Anywhere => {
                if cx.update(|window, _| {
                    window
                        .context_stack()
                        .iter()
                        .any(|context| context.contains(PROGRAM_CONTEXT))
                }) {
                    self.click_into(cx, "review-bar");
                }
            }
            // The bar of the review pane, drawn in both views, at its left end: a click
            // there puts the focus in the pane and pauses no follow.
            Place::In(yaar_review_pane::KEY_CONTEXT) => self.click_into(cx, "review-bar"),
            // The tree of Changes: Tab from the rows moves the focus into it, as a user
            // gets there by keys.
            Place::In(yaar_review_pane::HISTORY_CONTEXT) => {
                if !self.focus_is_in(yaar_review_pane::HISTORY_CONTEXT, cx) {
                    // The Keeper answers for the history after the rows are drawn, and
                    // Tab does nothing before the tree is there.
                    // Flow has no tree: a step before left it on screen, perhaps.
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
            // The view of the terminal on screen.
            Place::In(PROGRAM_CONTEXT) => self.click_into(cx, "terminal-slot-0"),
            // The list of references opens through the Navigator only: `g r` on a name of
            // a line, as a user opens it, and its keys work in it.
            Place::In(yaar_review_pane::REFERENCES_CONTEXT) => self.open_the_list_of_references(cx),
            Place::In(other) => panic!(
                "{} works in the context {other:?}, and the walk does not know where that is",
                control.title
            ),
        }
    }

    /// Presses the key of `control` where it works, as a user does. Fails when the
    /// control has no key, and when the key runs anything but the action of the control.
    fn press(&mut self, control: &Control, cx: &mut VisualTestContext) {
        assert!(
            !control.keystroke.is_empty(),
            "{} has no key",
            control.title
        );
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

    /// Closes what the answer of the Navigator to an earlier key opened: the file view, which
    /// takes the place of the rows, and the list of references, which takes the focus. Both
    /// come with an answer that arrives from another thread, so [`Composed::press`] waits
    /// for the answer of the keys that ask, and no key finds either there that it did not
    /// open itself.
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

    /// Waits for the answer to what the key of `control` asked of the Navigator, so that
    /// what it opens is open when the next step begins: the hover and the list of
    /// references are shown while they wait, and the keys that open a definition or a
    /// reference open the file view when the Navigator has read the file.
    fn wait_for_the_navigator(&mut self, control: &Control, cx: &mut VisualTestContext) {
        wait_until(cx, "the Navigator's answers to the pane's asks", |cx| {
            self.review.read_with(cx, |pane, _| {
                pane.hover().is_none_or(|hover| !hover.waiting)
                    && pane
                        .references()
                        .is_none_or(|references| !references.waiting)
            })
        });
        // The pane shows no sign of a definition that is on its way: the key cleared the
        // notice, so an answer shows as the list, the file view or a notice that says none
        // was found.
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

    /// The repository of the workspace on screen, by the Frame's list.
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

    /// Puts the reviewer on the added line of `notes.txt`, `2`, by a click on its row, in
    /// Changes: a line of the files on disk, whose server the pane's line tells of (a line
    /// the reviewer came to by itself may be a removed one, whose old side has no server
    /// before it is first used).
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

    /// Opens the list of references the way a user does: Changes, the reviewer on a name of
    /// a line of the notes, which the stand-in's row also serves, and the key of Find
    /// References; waits for the answer, which the Navigator finds by text match or from the
    /// server, and for the focus to be in the list.
    fn open_the_list_of_references(&mut self, cx: &mut VisualTestContext) {
        self.show_changes(cx);
        self.click_into_the_review_pane(cx);
        let mut row = None;
        wait_until(cx, "a line of the notes with a name drawn", |cx| {
            let rows = self.review.read_with(cx, |pane, _| pane.rows().len());
            row = (0..rows).find_map(|row| {
                ["new", "old"].into_iter().find_map(|side| {
                    cx.debug_bounds(leak(format!("review-text-{row}-{side}:three")))
                        .map(|_| (row, side))
                })
            });
            row.is_some()
        });
        let (row, side) = row.expect("a row");
        let selector = leak(format!("review-text-{row}-{side}:three"));
        let bounds = cx.debug_bounds(selector).expect("the text is drawn");
        // The first byte of the name, which is the whole line.
        cx.simulate_click(
            gpui_kit::point(bounds.left() + gpui_kit::px(2.), bounds.center().y),
            Modifiers::none(),
        );
        cx.simulate_keystrokes(&key_of("Find References"));
        wait_until(cx, "the list of references", |cx| {
            self.review.read_with(cx, |pane, _| {
                pane.references()
                    .is_some_and(|references| !references.waiting)
            })
        });
        assert!(
            self.focus_is_in(yaar_review_pane::REFERENCES_CONTEXT, cx),
            "the list of references took no focus"
        );
    }

    /// Fails when the Window is asking the user for something after the key of `control`,
    /// which says it asks for nothing. Read where a user meets it: the dialog for a
    /// folder is the platform's, and the fields for a line of text are drawn by the Frame
    /// and by the review pane under the selectors their docs name.
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
        assert!(
            cx.debug_bounds("review-commit-field").is_none(),
            "the key {} of {} says it asks for nothing and opened the field for a commit message",
            control.key(),
            control.title
        );
    }

    /// Gives what `control` asks the user for after its key.
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

    /// One step of the walk: for a kind of request a user starts, every control that
    /// says it sends that kind has its key pressed, is given what it asks for, and has
    /// to put that request, with what the walk chose for it, on the tap. Fails when no
    /// control sends the kind.
    fn walk(&mut self, kind: RequestKind, cx: &mut VisualTestContext) {
        self.walk_the_controls_titled(kind, |_| true, cx);
    }

    /// [`Composed::walk`] for the controls of the kind whose titles `chosen` accepts: for a
    /// test that needs one thing a kind of request does, not every control that does it.
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
            // The workspace to go back to after a step that had to bring another on screen,
            // and the stand-in a launcher started, which the step has to end.
            let mut back_to: Option<WorkspaceId> = None;
            let mut launched_after: Option<u32> = None;
            // No arm catches the rest: a kind added to the Link stops this from
            // compiling until somebody says here how to walk it.
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
                        // A shell for the workspace on screen, at whatever size the pane lays
                        // a terminal out at. Only the step that asks for a terminal sends one.
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
                    // The launcher of the chosen CLI, and the resume of the session the
                    // restart cut off, both in the workspace that session belongs to.
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
                    // A draft to delete, written the way a user writes one.
                    let (review, comment) = self.write_a_draft(cx);
                    let request = Request::DeleteDraft { review, comment };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::CopyReview => {
                    // A draft to copy, so that the Keeper has something to send.
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
                    // The question the Navigator asks before the project's own server runs,
                    // for a repository of its own so that every answer finds a question to
                    // answer, on the pane's line, as a user meets it.
                    back_to = self.on_screen(cx).map(|workspace| workspace.id);
                    let folder = self.new_repository();
                    // The pane may show Flow after an earlier step, which draws no first row.
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
                RequestKind::SetSetting => {
                    // The workspace on screen shows Flow as the timeline, which nobody chose:
                    // the key switches it to the live diff and keeps that for the workspace.
                    // The key acts in Flow only, so Flow is shown first, as a user does.
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
                | RequestKind::ExportVersion => {
                    panic!("the Window starts {kind:?} by itself: no user does")
                }
                RequestKind::Report => {
                    panic!("a piece of an agent CLI starts {kind:?}: no user does")
                }
                // The control says which answer it gives, and the Keeper is asked for pi's, the
                // one CLI of the world that asks.
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
                    // The thread is attached alone from the line it is on; the review whole
                    // by the other key.
                    let request = Request::AttachReview {
                        review,
                        thread: (control.title == "Attach Thread").then_some(thread),
                    };
                    (Answer::Nothing, Box::new(move |passed| *passed == request))
                }
                RequestKind::DetachReview => {
                    let (review, _) = self.agent_with_a_thread(cx);
                    // Both controls detach what is attached: the one at the agent's terminal
                    // is drawn, and acts, only while something is.
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
            };

            let from = self.tap.requests().len();
            self.press(&control, cx);
            self.give(&control, &answer, cx);
            // The Window writes a request before the action that asked returns, so it
            // is on its way by now. The folder and the branch are new with each step,
            // and a terminal is asked for by this step alone: no request of an earlier
            // step answers this wait.
            let passed = self.tap.wait_for_request(DEADLINE, |passed| wanted(passed));
            self.settle(kind, &passed, from, cx);
            if let Some(before) = launched_after {
                self.end_the_stand_in_started_after(before);
            }
            if let Some(workspace) = back_to {
                self.bring_on_screen(&workspace, cx);
            }

            // A folder that was opened comes on screen. The next step starts once it
            // has, so that it finds the workspace it acts on.
            if let Answer::Folder(folder) = &answer {
                self.wait_until_on_screen(folder, cx);
            }
        }
    }

    /// Puts `workspace` on screen by a click on its row of the sidebar, as a user does, and
    /// waits until it is. Gives the workspace that was on screen, or none when `workspace`
    /// was.
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

    /// Ends the stand-in a launcher of the Window started, which follows the script that
    /// holds at the gate of the world, and which no agent of the walk runs beside: the gate
    /// is made, the stand-in is gone, and the gate is taken away again for the agent the walk
    /// starts later. The stand-in is the one whose process differs from `before`, the last the
    /// log named, once the log names one; it names it after the stand-in read its script.
    fn end_the_stand_in_started_after(&mut self, before: u32) {
        let world = &self.world;
        assert!(
            yaar_system_tests::wait_until(DEADLINE, || {
                // A log read while the stand-in starts it is empty: pid 0.
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

    /// The control of the Window titled `title`.
    fn control_titled(title: &str) -> Control {
        yaar_window::controls()
            .into_iter()
            .find(|control| control.title == title)
            .unwrap_or_else(|| panic!("no control is titled {title:?}"))
    }

    /// The review of the agent and a thread of it with a draft, the reviewer on its line:
    /// the agent is started and the draft written the first time this is asked.
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

    /// Attaches the review of the agent by the key of Attach Review when nothing is
    /// attached to it, and waits until the review pane and the agent's terminal show it.
    fn make_sure_it_is_attached(&mut self, cx: &mut VisualTestContext) {
        let attached = self
            .review
            .read_with(cx, |pane, _| pane.review().and_then(|r| r.attached.clone()));
        if attached.is_none() {
            self.press(&Self::control_titled("Attach Review"), cx);
            self.wait_for_attachment(Some(Attachment::Review), cx);
        }
    }

    /// Puts the reviewer on the line the agent's thread sits on, by the keys a user uses:
    /// Changes shown, the pane focused, then Line Down or Line Up until the thread's row
    /// follows the row the reviewer is on. A key of an earlier step may have moved the
    /// reviewer, and a click on the first row would not put it back, since a click on a
    /// row that holds no line leaves the position where it was. The keys that act on the
    /// thread on the line find it only there.
    fn put_the_reviewer_on_the_thread(&mut self, cx: &mut VisualTestContext) {
        let thread = self
            .agent()
            .thread
            .clone()
            .expect("the walk wrote a thread");
        self.show_changes(cx);
        self.click_into_the_review_pane(cx);
        // The row of the thread, once the pane draws it, and the row the reviewer is on.
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

    /// Ends the agent the walk started: the gate is made, and the stand-in ends its session
    /// and itself.
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

    /// The last step of the walk: every control that says it sends the stop-everything
    /// frame has its key pressed, where it works, and the frame has to pass the tap. The
    /// frame ends the Keeper of the world, and a user has nothing left to do after it, so
    /// this comes after every other step, and the step is over only when the Keeper is gone.
    fn walk_the_stop(&mut self, cx: &mut VisualTestContext) {
        let controls = controls_that_stop();
        assert!(
            !controls.is_empty(),
            "no control of the Window sends the stop-everything frame: a user has no way to start it"
        );
        // The programs of the Keeper are listed while it runs: once it is gone they belong
        // to the system, and nothing says whose they were.
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
            assert!(
                yaar_system_tests::wait_until(DEADLINE, || world.keeper_pid().is_none()),
                "the Keeper still runs after the key {} of {}",
                control.key(),
                control.title
            );
        }
        // The stop frame has the Keeper end what it holds, the shell the walk opened with
        // New Terminal among it.
        assert!(
            yaar_system_tests::wait_until(DEADLINE, || programs.iter().all(|pid| !is_alive(*pid))),
            "the programs of the Keeper still run after the stop: {:?}",
            programs
                .iter()
                .filter(|pid| is_alive(**pid))
                .collect::<Vec<_>>()
        );
    }

    fn wait_until_on_screen(&mut self, folder: &Path, cx: &mut VisualTestContext) {
        wait_until(cx, "the opened folder on screen", |cx| {
            self.on_screen(cx)
                .is_some_and(|workspace| workspace.path == folder)
        });
    }

    /// Sets the base of the workspace on screen to `base` as a user does, by the control
    /// that sends that request, and waits until the request has passed the tap. Returns
    /// its place among the requests the tap lists, or none when the stop-everything frame
    /// passes the tap first: it ends the Keeper, and the request would never pass.
    fn set_base(&mut self, base: &str, cx: &mut VisualTestContext) -> Option<usize> {
        let control = controls_that_send(RequestKind::SetBase)
            .into_iter()
            .next()
            .expect("a control of the Window sends SetBase");
        // The request is looked for from here on: an earlier one that set a base of the
        // same name is not the one this call sent.
        let from = self.tap.requests().len();
        self.press(&control, cx);
        self.give(&control, &Answer::Text(base.to_owned()), cx);
        let sets_it = |request: &Request| match request {
            Request::SetBase { base: set, .. } => set == base,
            _ => false,
        };
        let tap = &self.tap;
        let came = yaar_system_tests::wait_until(DEADLINE, || {
            tap.requests()[from..].iter().any(sets_it) || tap.wait_for_stop(Duration::ZERO)
        });
        // The stop frame is latched, so it is read after the wait whichever event ended
        // it: a Keeper on its way out may still let the request through the tap, and
        // only the key of this step can have sent the frame.
        if self.tap.wait_for_stop(Duration::ZERO) {
            return None;
        }
        assert!(
            came,
            "neither the request that sets the base {base} nor the stop-everything frame \
             passed the tap; the requests listed are {:?}",
            &self.tap.requests()[from..]
        );
        self.tap.wait_for_request(DEADLINE, sets_it);
        let found = self.tap.requests()[from..]
            .iter()
            .position(sets_it)
            .expect("the tap lists the request it waited for");
        Some(from + found)
    }
}

#[gpui_kit::test]
fn every_request_a_user_starts_has_a_control_whose_key_sends_it(cx: &mut TestAppContext) {
    let (mut window, mut cx) = open_composed_for_the_walk(cx);

    // A workspace is on screen before the walk, whichever kind the walk meets first:
    // without one there is nothing to set a base for.
    window.walk(RequestKind::OpenFolder, &mut cx);

    // Every kind there is, the kinds of the reviews among them: the first of those starts
    // the agent whose review they act on.
    for kind in RequestKind::started_by_user() {
        window.walk(kind, &mut cx);
    }
    window.end_the_agent();

    // The stop frame is no request. It ends the Keeper, so it is pressed last.
    window.walk_the_stop(&mut cx);
    // The servers the answers started stop with the Navigator.
    window.end_the_navigator(&mut cx);
}

/// Presses the key of every control that names no message, one step for each, and checks
/// that none of them sent a request a user starts. `ready` puts the Window where the keys
/// act, before each step: a key of an earlier step may have moved the Window on.
fn walk_the_silent_keys(
    window: &mut Composed,
    cx: &mut VisualTestContext,
    mut ready: impl FnMut(&mut Composed, &mut VisualTestContext),
) {
    let mut silent: Vec<Control> = yaar_window::controls()
        .into_iter()
        .filter(|control| control.sends.is_none())
        .collect();
    // Quit comes last: in the app nothing comes after it. The test platform does not
    // end the app, so the window is still there to close the step.
    let quit = yaar_window::Quit.name();
    silent.sort_by_key(|control| (control.action)().name() == quit);
    assert_eq!(
        silent.last().map(|control| (control.action)().name()),
        Some(quit)
    );

    // This shows an absence, and is as strong as its end: each step is closed by a
    // request the walk starts itself, a base named for the step, and whatever the key
    // had the Window send lies before that request on the one connection. What a key
    // would set off only after an answer of the Keeper to something later is not seen.
    //
    // It sees what a key sends with no further input, and three of the requests a user
    // starts go out only after the user gave something: a folder in the dialog, a line
    // in the field for a base or for a commit message. So each key must also leave the
    // dialog and both fields closed. A key that asked in a way
    // the walk does not know of, and sent after an answer there, would pass.
    // Resume Follow acts in Flow and sends nothing in Changes either: the walk presses it
    // in both views, the second time after the first one, with Changes shown.
    let resume = yaar_review_pane::ResumeFollow.name();
    let mut steps: Vec<(Control, bool)> = Vec::new();
    for control in silent {
        steps.push((control, false));
        if (control.action)().name() == resume {
            steps.push((control, true));
        }
    }
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
        window.press(&control, cx);
        // Checked before the step is closed: the closing types into the field for the
        // base, and would hand a field this key had opened its line all the same.
        window.assert_asks_nothing(&control, cx);
        // The keys of Comment, Edit Draft and Reply may open the box, which takes the
        // focus: the keys of the steps after would be typed into it as text. Nothing was
        // typed, so no save went out.
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
        // No such branch exists. The Keeper refuses it, which is all the same here: the
        // request has passed the tap.
        // The stop frame is no request, so the loop below does not see it. A key that
        // sent it would end the Keeper, and the request that closes the step would never
        // pass: the step is closed on that frame as well, and it is told apart here.
        let Some(closed) = window.set_base(&format!("after-{}-{step}", control.key()), cx) else {
            panic!(
                "the key {} of {} names no message and sent the stop-everything frame",
                control.key(),
                control.title
            );
        };
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

    // The window stood through the walk with what it was opened with.
    assert_eq!(
        window.frame.read_with(&*cx, |frame, _| frame.panes().len()),
        yaar_window::panes().len()
    );
}

#[gpui_kit::test]
fn no_key_of_a_control_that_names_no_message_sends_what_a_user_starts(cx: &mut TestAppContext) {
    // What a control says it sends is something a user starts.
    for control in yaar_window::controls() {
        match control.sends {
            Some(Sends::Request(kind)) => assert_eq!(
                kind.origin(),
                Origin::User,
                "{} says it sends {kind:?}",
                control.title
            ),
            // The stop-everything frame is no request, and has no origin to check. Its
            // control is pressed by the forward walk, last, where the tap sees the frame.
            Some(Sends::StopEverything) => {}
            None => {}
        }
    }

    let (mut window, mut cx) = open_composed(cx);
    // Two workspaces, so that the keys that change the one on screen have one to change
    // to, and so set off the questions the Window asks by itself.
    window.walk(RequestKind::OpenFolder, &mut cx);
    window.walk(RequestKind::OpenFolder, &mut cx);
    // A terminal on screen, opened by its key as a user opens one, so that the keys of the
    // terminal pane have a terminal to act on and the focus is inside one.
    // A shell: the launchers and the resume have their own walk, in the test above.
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
    // One workspace: the keys that change the workspace on screen would take the agent away
    // from the keys after them. The agent runs in it, its review has a draft on a thread
    // and is attached, so that the keys of attaching and of the agent's terminal are
    // pressed where they act and not where they have nothing to do.
    window.walk(RequestKind::OpenFolder, &mut cx);
    window.agent_with_a_thread(&mut cx);
    window.make_sure_it_is_attached(&mut cx);

    // A key of an earlier step may have gone on to another review, closed the view of the
    // attachment or the like: the review of the agent is back, and attached, before each.
    walk_the_silent_keys(&mut window, &mut cx, |window, cx| {
        // What an earlier key's answer opened would take the keys that follow.
        window.leave_what_the_navigator_opened(cx);
        let session = window.agent().session.clone();
        window.show_the_review_of(&session, cx);
        window.make_sure_it_is_attached(cx);
        // The keys that act on the thread on the reviewer's line, Original Line, Comment,
        // Edit Draft and Reply among them, are pressed with the reviewer on it.
        window.put_the_reviewer_on_the_thread(cx);
    });
    window.end_the_agent();
    window.end_the_navigator(&mut cx);
}
