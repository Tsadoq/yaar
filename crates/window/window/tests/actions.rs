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

use gpui_kit::{Action, Modifiers, TestAppContext, VisualTestContext, point, px};
use yaar_frame::{Asks, Control, PROGRAM_CONTEXT, Place, Sends};
use yaar_link::{Origin, Program, Request, RequestKind, Workspace};
use yaar_system_tests::DEADLINE;

mod common;

use common::{Composed, wait_until};

/// Which request a step of the walk waits for on the tap.
type Wanted = Box<dyn Fn(&Request) -> bool>;

/// The kinds of request a user starts that the walk leaves out while the slice that adds
/// them builds their controls: the reviews' own. The contract of the slice added them to
/// the Link before any control sends them; the package that composes the Window teaches
/// the walk each of them and empties this list, and the slice is not closed before it is
/// empty.
const WALKED_LATER: &[RequestKind] = &[
    RequestKind::DeleteDraft,
    RequestKind::CopyReview,
    RequestKind::MarkReviewed,
    RequestKind::SetViewed,
    RequestKind::Commit,
];

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
/// in the table of controls.
fn open_composed(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let (window, cx) = common::open_composed(cx, Vec::new());
    let listed: Vec<&str> = yaar_window::panes().iter().map(|pane| pane.name).collect();
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.panes()),
        listed
    );
    (window, cx)
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
        let repository = self.world.repository(&name);
        repository.git(&["checkout", "--quiet", "-b", "work"]);
        repository.write("notes.txt", "one\ntwo\nthree\n");
        repository.commit("Add the notes");
        repository.write("notes.txt", "one\n2\nthree\n");
        let folder = repository.path().to_owned();
        self.repositories.push(repository);
        folder
    }

    /// Makes a new branch in the repository of the workspace on screen, and returns that
    /// workspace with the name of the branch.
    fn new_branch_on_screen(&mut self, cx: &mut VisualTestContext) -> (Workspace, String) {
        let on_screen = self
            .on_screen(cx)
            .expect("a workspace is on screen before the walk sets a base");
        let branch = format!("base-{}", self.next());
        self.repositories
            .iter()
            .find(|repository| repository.path() == on_screen.path)
            .expect("the workspace on screen is one the walk opened")
            .git(&["branch", &branch]);
        (on_screen, branch)
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
                    self.click_into(cx, "review-row-0");
                }
            }
            // The first row of the Review pane, at its left end: an offer to set a base,
            // where the row holds one, lies further right.
            Place::In(yaar_review_pane::KEY_CONTEXT) => self.click_into(cx, "review-row-0"),
            // The view of the terminal on screen.
            Place::In(PROGRAM_CONTEXT) => self.click_into(cx, "terminal-slot-0"),
            Place::In(other) => panic!(
                "{} works in the context {other:?}, and the walk does not know where that is",
                control.title
            ),
        }
    }

    /// Clicks at the left end of an element, once it is drawn.
    fn click_into(&mut self, cx: &mut VisualTestContext, selector: &'static str) {
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

    /// Presses the key of `control` where it works, as a user does. Fails when the
    /// control has no key, and when the key runs anything but the action of the control.
    fn press(&mut self, control: &Control, cx: &mut VisualTestContext) {
        assert!(
            !control.keystroke.is_empty(),
            "{} has no key",
            control.title
        );
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
    }

    /// Fails when the Window is asking the user for something after the key of `control`,
    /// which says it asks for nothing. Read where a user meets it: the dialog for a
    /// folder is the platform's, and the field for a line of text is drawn by the Frame
    /// under the selector its docs name.
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
        let controls = controls_that_send(kind);
        assert!(
            !controls.is_empty(),
            "no control of the Window sends {kind:?}: a user has no way to start it"
        );
        for control in controls {
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
                RequestKind::OpenTerminal => {
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
                RequestKind::DeleteDraft
                | RequestKind::CopyReview
                | RequestKind::MarkReviewed
                | RequestKind::SetViewed
                | RequestKind::Commit => {
                    panic!("{kind:?} is not walked yet: see WALKED_LATER")
                }
                RequestKind::ListWorkspaces
                | RequestKind::GetChanges
                | RequestKind::ReadFile
                | RequestKind::ResizeTerminal
                | RequestKind::GetReviews
                | RequestKind::SaveDraft => {
                    panic!("the Window starts {kind:?} by itself: no user does")
                }
            };

            self.press(&control, cx);
            self.give(&control, &answer, cx);
            // The Window writes a request before the action that asked returns, so it
            // is on its way by now. The folder and the branch are new with each step,
            // and a terminal is asked for by this step alone: no request of an earlier
            // step answers this wait.
            self.tap.wait_for_request(DEADLINE, |passed| wanted(passed));

            // A folder that was opened comes on screen. The next step starts once it
            // has, so that it finds the workspace it acts on.
            if let Answer::Folder(folder) = &answer {
                self.wait_until_on_screen(folder, cx);
            }
        }
    }

    fn wait_until_on_screen(&mut self, folder: &Path, cx: &mut VisualTestContext) {
        wait_until(cx, "the opened folder on screen", |cx| {
            self.on_screen(cx)
                .is_some_and(|workspace| workspace.path == folder)
        });
    }

    /// Sets the base of the workspace on screen to `base` as a user does, by the control
    /// that sends that request, and waits until the request has passed the tap. Returns
    /// its place among the requests the tap lists.
    fn set_base(&mut self, base: &str, cx: &mut VisualTestContext) -> usize {
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
        self.tap.wait_for_request(DEADLINE, sets_it);
        let found = self.tap.requests()[from..]
            .iter()
            .position(sets_it)
            .expect("the tap lists the request it waited for");
        from + found
    }
}

#[gpui_kit::test]
fn every_request_a_user_starts_has_a_control_whose_key_sends_it(cx: &mut TestAppContext) {
    let (mut window, mut cx) = open_composed(cx);

    // A workspace is on screen before the walk, whichever kind the walk meets first:
    // without one there is nothing to set a base for.
    window.walk(RequestKind::OpenFolder, &mut cx);

    for kind in RequestKind::started_by_user().filter(|kind| !WALKED_LATER.contains(kind)) {
        window.walk(kind, &mut cx);
    }
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
            // No control sends the stop-everything frame yet. It joins the walk when
            // its control is built, and the tap, which keeps requests, has to keep
            // that frame too by then.
            Some(Sends::StopEverything) => panic!(
                "{} sends the stop-everything frame, which the walk does not follow yet",
                control.title
            ),
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
    window.walk(RequestKind::OpenTerminal, &mut cx);
    wait_until(&mut cx, "the terminal on screen", |cx| {
        cx.debug_bounds("terminal-slot-0").is_some()
    });

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
    // It sees what a key sends with no further input, and both requests a user starts
    // so far go out only after the user gave something: a folder in the dialog, a line
    // in the field. So each key must also leave both closed. A key that asked in a way
    // the walk does not know of, and sent after an answer there, would pass.
    for control in silent {
        assert_eq!(
            control.asks,
            Asks::Nothing,
            "{} asks the user for something, and the walk does not know what to give",
            control.title
        );
        let before = window.tap.requests().len();
        window.press(&control, &mut cx);
        // Checked before the step is closed: the closing types into the field for the
        // base, and would hand a field this key had opened its line all the same.
        window.assert_asks_nothing(&control, &mut cx);
        // No such branch exists. The Keeper refuses it, which is all the same here: the
        // request has passed the tap.
        let closed = window.set_base(&format!("after-{}", control.key()), &mut cx);

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
        window.frame.read_with(&cx, |frame, _| frame.panes().len()),
        yaar_window::panes().len()
    );
}
