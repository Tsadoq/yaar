//! Component test: the keys the toolkit binds for the whole window reach a program, and
//! the Frame's kept keys and its other keys behave as the places of their rows say, in a
//! window that holds an element that sets the context of a program.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    Keystroke, OperatingSystem, ParentElement, Pixels, Point, Render, Size, Styled, TestAppContext,
    VisualTestContext, Window, WindowBounds, WindowOptions, actions, div, px,
};
use yaar_frame::{
    Asks, Control, KEY_CONTEXT, PROGRAM_CONTEXT, PROGRAM_KEYS, Place, bind, controls,
    release_program_keys,
};

actions!(
    program_keys_test,
    [
        /// An action of the test's own, bound as a kept key and as a key that works
        /// anywhere.
        Mark
    ]
);

/// A keystroke as the keys of the app are written.
fn written(keystroke: &Keystroke) -> String {
    keystroke.unparse(if cfg!(target_os = "macos") {
        OperatingSystem::Mac
    } else {
        OperatingSystem::Linux
    })
}

/// The keys a program's element heard, as written, and the actions that ran for keys of
/// the window, by name; both in order.
#[derive(Default)]
struct Heard {
    keys: Vec<String>,
    actions: Vec<(String, Option<&'static str>)>,
}

/// What stands where a terminal does: an element in the context of a program, inside the
/// context of the Frame, which takes the focus and records every key it is handed,
/// without taking it from anyone else.
struct Program {
    focus: FocusHandle,
    heard: Rc<RefCell<Heard>>,
}

impl Render for Program {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().key_context(KEY_CONTEXT).child(
            div()
                .id("program")
                .size_full()
                .key_context(PROGRAM_CONTEXT)
                .track_focus(&self.focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    this.heard.borrow_mut().keys.push(written(&event.keystroke));
                    // The key goes on, as a view that takes a key does not do here: what
                    // the toolkit does with it afterwards is not in question.
                    cx.propagate();
                })),
        )
    }
}

const WINDOW: Size<Pixels> = Size {
    width: px(400.),
    height: px(300.),
};

/// A window of the toolkit with the element of a program focused, in an app prepared as
/// the Window program prepares it, with the keys of the Frame bound and the toolkit's own
/// keys given back to a program as `release` says.
fn open_program(
    release: bool,
    extra: &[Control],
    cx: &mut TestAppContext,
) -> (Rc<RefCell<Heard>>, VisualTestContext) {
    let heard = Rc::new(RefCell::new(Heard::default()));
    cx.update(|cx| {
        gpui_kit::init(cx);
        if release {
            release_program_keys(cx);
        }
        bind(&controls(), cx);
        bind(extra, cx);
        // An action that nothing handles is no action to the observer.
        cx.on_action(|_: &Mark, _| {});
        let seen = heard.clone();
        cx.observe_keystrokes(move |event, _, _| {
            seen.borrow_mut().actions.push((
                written(&event.keystroke),
                event.action.as_ref().map(|action| action.name()),
            ));
        })
        .detach();
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: WINDOW,
        })),
        ..Default::default()
    };
    let (window, program) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, {
                let heard = heard.clone();
                move |_, cx| {
                    cx.new(|cx| Program {
                        focus: cx.focus_handle(),
                        heard,
                    })
                }
            })
        })
        .expect("open a test window");
    let mut cx = VisualTestContext::from_window(window, cx);
    program.update_in(&mut cx, |program, window, cx| {
        program.focus.focus(window, cx)
    });
    cx.run_until_parked();
    (heard, cx)
}

#[gpui_kit::test]
fn each_key_the_toolkit_binds_for_the_whole_window_reaches_a_program_and_runs_no_action(
    cx: &mut TestAppContext,
) {
    let (heard, mut cx) = open_program(true, &[], cx);

    for key in PROGRAM_KEYS {
        cx.simulate_keystrokes(key);
        let heard = heard.borrow();
        assert_eq!(
            heard.keys.last().map(String::as_str),
            Some(*key),
            "the program's element heard the key {key}"
        );
        assert_eq!(
            heard.actions.last(),
            Some(&((*key).to_owned(), None)),
            "the key {key} ran no action"
        );
    }
    assert_eq!(heard.borrow().keys.len(), PROGRAM_KEYS.len());
}

#[gpui_kit::test]
fn without_the_keys_given_back_the_toolkit_takes_tab_from_a_program(cx: &mut TestAppContext) {
    // The control of the test above: the toolkit's own binding is what the call undoes.
    let (heard, mut cx) = open_program(false, &[], cx);

    cx.simulate_keystrokes("tab");
    let heard = heard.borrow();
    assert!(heard.keys.is_empty(), "the program heard {:?}", heard.keys);
    assert!(
        matches!(heard.actions.last(), Some((key, Some(_))) if key == "tab"),
        "tab ran an action of the toolkit: {:?}",
        heard.actions
    );
}

#[gpui_kit::test]
fn inside_a_program_a_kept_key_runs_its_action_and_a_key_that_works_anywhere_only_on_macos(
    cx: &mut TestAppContext,
) {
    let extra = [
        Control {
            title: "Kept",
            action: || Box::new(Mark),
            keystroke: "secondary-j",
            place: Place::Kept,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Anywhere",
            action: || Box::new(Mark),
            keystroke: "secondary-k",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
    ];
    let (heard, mut cx) = open_program(true, &extra, cx);

    let ran = |key: &str, heard: &Rc<RefCell<Heard>>| {
        heard
            .borrow()
            .actions
            .iter()
            .rev()
            .find(|(pressed, _)| pressed == key)
            .map(|(_, action)| *action)
    };
    cx.simulate_keystrokes(&extra[0].key());
    assert_eq!(
        ran(&extra[0].key(), &heard),
        Some(Some("program_keys_test::Mark")),
        "a kept key runs its action inside a program, on every system"
    );
    cx.simulate_keystrokes(&extra[1].key());
    let expected = cfg!(target_os = "macos").then_some("program_keys_test::Mark");
    assert_eq!(
        ran(&extra[1].key(), &heard),
        Some(expected),
        "a key that works anywhere runs inside a program on macOS only"
    );
    if !cfg!(target_os = "macos") {
        // The key that no action took went on to the program.
        assert!(heard.borrow().keys.contains(&extra[1].key()));
    }
}
