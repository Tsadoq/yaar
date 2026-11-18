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

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    Action, Entity, KeystrokeEvent, Modifiers, Subscription, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_frame::{Asks, Control, Frame, LinkStatus, Models, Opened, Sends};
use yaar_link::{Origin, Request, RequestKind, Workspace};
use yaar_system_tests::{DEADLINE, Repo, Tap, World};

/// The composed Window of one test, in a world of its own, with a tap on its link.
struct Composed {
    // The tap goes before the world it leads into: fields are dropped in this order.
    tap: Tap,
    world: World,
    frame: Entity<Frame>,
    models: Models,
    /// The actions the keystrokes ran since the list was last taken, by name, as the
    /// test platform tells them.
    ran: Rc<RefCell<Vec<&'static str>>>,
    _watching_the_keys: Subscription,
    /// The repositories the walk made, each opened as a workspace.
    repositories: Vec<Repo>,
    /// Counts what the walk makes, for names of its own.
    made: u32,
}

/// What the walk has to give when a control asks the user for something.
#[derive(Debug)]
enum Answer {
    /// The folder to choose in the dialog.
    Folder(PathBuf),
    /// The line to type into the field, before Enter.
    Text(String),
}

/// Opens the Window the way the program does, with [`yaar_window::init`] and
/// [`yaar_window::open`], on the socket of a tap that leads to the Keeper of a new world,
/// and waits until that Keeper has answered.
fn open_composed(cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let tap = world.tap();

    // A real socket and a real process: the connection is read by a thread, and answered
    // by a program, that the test executor cannot see.
    cx.executor().allow_parking();
    let ran: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let watching = cx.update(|cx| {
        yaar_window::init(cx);
        let ran = ran.clone();
        cx.observe_keystrokes(move |event: &KeystrokeEvent, _, _| {
            if let Some(action) = &event.action {
                ran.borrow_mut().push(action.name());
            }
        })
    });
    let Opened {
        window,
        frame,
        models,
    } = cx
        .update(|cx| yaar_window::open(tap.socket(), cx))
        .expect("open the composed Window on the test platform");
    let mut cx = VisualTestContext::from_window(window, cx);

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
    // Every pane of the list is in the window, as well as in the table of controls.
    let listed: Vec<&str> = yaar_window::panes().iter().map(|pane| pane.name).collect();
    assert_eq!(frame.read_with(&cx, |frame, _| frame.panes()), listed);

    (
        Composed {
            tap,
            world,
            frame,
            models,
            ran,
            _watching_the_keys: watching,
            repositories: Vec::new(),
            made: 0,
        },
        cx,
    )
}

/// Keeps the app running until `holds` says yes. Fails at the deadline, naming `what`.
fn wait_until(
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
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
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

    /// A number no earlier call gave, for a name of the walk's own.
    fn next(&mut self) -> u32 {
        self.made += 1;
        self.made
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

    /// Puts the focus where the key of `control` works. A key with no context works
    /// wherever the focus is, and is pressed where the focus happens to be.
    fn focus_where_the_key_works(&mut self, control: &Control, cx: &mut VisualTestContext) {
        match control.context {
            None => {}
            Some(yaar_review_pane::KEY_CONTEXT) => {
                // The first row of the Review pane, at its left end: an offer to set a
                // base, where the row holds one, lies further right.
                let mut row = None;
                wait_until(cx, "the first row of the Review pane", |cx| {
                    row = cx.debug_bounds("review-row-0");
                    row.is_some()
                });
                let row = row.expect("the row is drawn");
                cx.simulate_click(
                    point(row.left() + px(2.), row.center().y),
                    Modifiers::none(),
                );
            }
            Some(other) => panic!(
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
        self.focus_where_the_key_works(control, cx);
        self.ran.borrow_mut().clear();
        cx.simulate_keystrokes(control.keystroke);
        assert_eq!(
            self.ran.take(),
            [(control.action)().name()],
            "what the key {} of {} ran",
            control.keystroke,
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
            control.keystroke,
            control.title
        );
        assert!(
            cx.debug_bounds("frame-base-field").is_none(),
            "the key {} of {} says it asks for nothing and opened the field for a line of text",
            control.keystroke,
            control.title
        );
    }

    /// Gives what `control` asks the user for after its key.
    fn give(&mut self, control: &Control, answer: &Answer, cx: &mut VisualTestContext) {
        match (control.asks, answer) {
            (Asks::Folder, Answer::Folder(folder)) => {
                assert!(
                    cx.did_prompt_for_paths(),
                    "the key {} of {} opens the dialog for a folder",
                    control.keystroke,
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
            let (answer, request) = match kind {
                RequestKind::OpenFolder => {
                    let folder = self.new_repository();
                    (
                        Answer::Folder(folder.clone()),
                        Request::OpenFolder { path: folder },
                    )
                }
                RequestKind::SetBase => {
                    let (on_screen, branch) = self.new_branch_on_screen(cx);
                    (
                        Answer::Text(branch.clone()),
                        Request::SetBase {
                            workspace: on_screen.id,
                            base: branch,
                        },
                    )
                }
                RequestKind::ListWorkspaces | RequestKind::GetChanges | RequestKind::ReadFile => {
                    panic!("the Window starts {kind:?} by itself: no user does")
                }
            };

            self.press(&control, cx);
            self.give(&control, &answer, cx);
            // The Window writes a request before the action that asked returns, so it
            // is on its way by now. The folder and the branch are new with each step:
            // no request of an earlier step answers this wait.
            self.tap
                .wait_for_request(DEADLINE, |passed| *passed == request);

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

    for kind in RequestKind::started_by_user() {
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
        let closed = window.set_base(&format!("after-{}", control.keystroke), &mut cx);

        let passed = window.tap.requests();
        for request in &passed[before..closed] {
            assert_ne!(
                request.kind().origin(),
                Origin::User,
                "the key {} of {} names no message and sent {request:?}",
                control.keystroke,
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
